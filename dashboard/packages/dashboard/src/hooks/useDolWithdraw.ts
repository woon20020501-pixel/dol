"use client";

import { useState, useCallback, useEffect } from "react";
import {
  useAccount,
  useWriteContract,
  useWaitForTransactionReceipt,
  useReadContract,
  usePublicClient,
} from "wagmi";
import { baseSepolia } from "wagmi/chains";
import { decodeEventLog, type Abi } from "viem";
import { getPBondConfig } from "@/lib/pbond";
import { getVaultConfig } from "@/lib/vault";
import {
  parseLocalStorageArray,
  isValidPendingRedeem,
  isValidAddress,
} from "@/lib/guards";
import { emitDolTxConfirmed } from "@/lib/txEvents";
import { log } from "@/lib/logger";

/**
 * useDolWithdraw — withdraw flow targeting pBondSenior (not the underlying
 * vault). User burns pBS shares and eventually receives USDC back.
 *
 * pBondSenior API (from ABI):
 *   - redeem(uint256 shares) → uint256 requestId   // step 1: queue
 *   - claimRedeem(uint256 requestId)               // step 2: after cooldown
 *   - redeemRequests(uint256) → (address, uint256, bool)
 *
 * Cooldown: read on-chain if the wrapper exposes it; otherwise 24h fallback.
 * Plan A will add cooldownSeconds() — for now we try to read it and fall
 * back gracefully.
 */

const SHARE_DECIMALS = 6;
const TARGET_CHAIN_ID = baseSepolia.id;
const STORAGE_KEY = "dol_pending_redeems";
// Plan A sets on-chain cooldown to 1800s (30min). Fallback only fires
// if the on-chain read fails (RPC hiccup, stale config).
const FALLBACK_COOLDOWN_MS = 30 * 60 * 1000;

export type PendingRedeem = {
  requestId: string;
  shares: number; // human-readable pBS
  requestedAt: number; // unix ms
};

function loadPending(user: string): PendingRedeem[] {
  try {
    const raw = localStorage.getItem(`${STORAGE_KEY}_${user}`);
    // Validated parse — discards malformed or injected entries
    return parseLocalStorageArray(raw, isValidPendingRedeem) as PendingRedeem[];
  } catch {
    return [];
  }
}

function savePending(user: string, reqs: PendingRedeem[]) {
  try {
    localStorage.setItem(`${STORAGE_KEY}_${user}`, JSON.stringify(reqs));
  } catch {
    // storage unavailable
  }
}

export function useDolWithdraw() {
  const config = getPBondConfig();
  const vault = getVaultConfig();
  const { address: userAddress, isConnected } = useAccount();
  const publicClient = usePublicClient({ chainId: TARGET_CHAIN_ID });
  const senior = config.senior;

  // ── Cooldown — read from the underlying vault (not pBondSenior) ───
  // Plan A exposes `cooldownSeconds()` on the vault itself. pBondSenior
  // doesn't have the function on its ABI, so earlier reads silently
  // failed and fell back to 24h. Now read from the vault directly.
  const { data: cooldownSecondsRaw } = useReadContract({
    address: vault?.address,
    abi: vault?.abi as Abi | undefined,
    functionName: "cooldownSeconds",
    chainId: TARGET_CHAIN_ID,
    query: { enabled: !!vault?.address, retry: false },
  });
  const cooldownMs =
    typeof cooldownSecondsRaw === "bigint"
      ? Number(cooldownSecondsRaw) * 1000
      : FALLBACK_COOLDOWN_MS;

  // ── Pending redeem list (localStorage) ─────────────────────────────
  const [pending, setPending] = useState<PendingRedeem[]>([]);

  useEffect(() => {
    if (userAddress) {
      setPending(loadPending(userAddress));
    } else {
      setPending([]);
    }
  }, [userAddress]);

  // Cross-tab sync: if another tab modifies the pending-redeems key
  // (e.g. user claims in tab B), this tab's list must re-load from
  // localStorage so it stops showing a claimed item as "Ready to
  // claim." The `storage` event fires ONLY on other tabs/windows,
  // never on the one that wrote, which is exactly what we want.
  useEffect(() => {
    if (!userAddress) return;
    const targetKey = `${STORAGE_KEY}_${userAddress}`;
    const onStorage = (e: StorageEvent) => {
      if (e.key === targetKey) {
        setPending(loadPending(userAddress));
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [userAddress]);

  // ── Recovery scanner — find orphan on-chain redeems not in localStorage
  // Fires once on mount when authenticated. Useful when the event decoder
  // missed a prior request (e.g. from the pre-fix bug where the decoder
  // looked for wrong arg names). Scans nextRedeemId → 0 and picks any
  // unclaimed requests where `user == currentUser` and adds them to local
  // pending list with a "already ready" countdown seed.
  useEffect(() => {
    if (!userAddress || !senior.address || !publicClient) return;
    let cancelled = false;

    (async () => {
      try {
        const nextId = (await publicClient.readContract({
          address: senior.address,
          abi: senior.abi as Abi,
          functionName: "nextRedeemId",
        })) as bigint;

        if (nextId === BigInt(0)) return;

        // Scan backwards; cap at 20 iterations for safety
        const scanFrom = Number(nextId) - 1;
        const scanTo = Math.max(0, scanFrom - 20);
        const foundOrphans: PendingRedeem[] = [];

        for (let i = scanFrom; i >= scanTo; i--) {
          if (cancelled) return;
          try {
            const result = (await publicClient.readContract({
              address: senior.address,
              abi: senior.abi as Abi,
              functionName: "redeemRequests",
              args: [BigInt(i)],
            })) as readonly [string, bigint, boolean];
            const [reqUser, , claimed] = result;
            if (
              !claimed &&
              reqUser.toLowerCase() === userAddress.toLowerCase()
            ) {
              foundOrphans.push({
                requestId: String(i),
                shares: 0, // unknown from struct, display TBD
                // Seed as "ready now" — we don't know real requestedAt.
                // Worst case: user sees "Ready" immediately on recovered ones.
                requestedAt: Date.now() - cooldownMs - 1000,
              });
            }
          } catch {
            // id doesn't exist, skip
          }
        }

        if (cancelled || foundOrphans.length === 0) return;

        setPending((prev) => {
          // Merge, dedup by requestId
          const existingIds = new Set(prev.map((p) => p.requestId));
          const newOnes = foundOrphans.filter(
            (f) => !existingIds.has(f.requestId),
          );
          if (newOnes.length === 0) return prev;
          const next = [...prev, ...newOnes];
          savePending(userAddress, next);
          return next;
        });
      } catch (e) {
        // Dev-only — recovery scan failures are non-fatal (user can
        // still withdraw; the scanner just can't cross-reference
        // orphan requests). Don't spam the production console.
        log.warn("[useDolWithdraw] Recovery scan failed:", e);
      }
    })();

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userAddress, senior.address]);

  // ── Request redeem (pBondSenior.redeem) ────────────────────────────
  const {
    writeContract: doRequest,
    data: requestHash,
    error: requestError,
    isPending: isRequestPending,
    reset: resetRequest,
  } = useWriteContract();

  const { data: requestReceipt, isLoading: isRequestConfirming, isSuccess: isRequestConfirmed } =
    useWaitForTransactionReceipt({
      hash: requestHash,
      chainId: TARGET_CHAIN_ID,
    });

  // Parse RedeemRequested event from the receipt and persist the id.
  // Dol contract event signature:
  //   RedeemRequested(address user indexed, uint256 dolBurned, uint256 redeemId)
  useEffect(() => {
    if (!isRequestConfirmed || !requestReceipt || !userAddress) return;

    // Event decoder hardening — fuzz + spoofing defenses.
    //
    // A tx receipt contains `logs[]` from EVERY contract touched in
    // the tx chain, not just ours. An attacker-controlled contract
    // in the call graph could emit a log with the exact same topic
    // signature as `RedeemRequested(address,uint256,uint256)` and
    // spoof a fake pending redeem entry in the user's UI. When the
    // user later taps "Claim" on that fake entry, they'd send tx to
    // an id that never existed → revert at best, or a confusing
    // state mismatch at worst.
    //
    // Defenses applied here:
    //   1. Only trust logs where `log.address === senior.address`.
    //      Anything emitted by a different contract is ignored.
    //   2. Only accept the `RedeemRequested` event name. Typo-bots
    //      emitting similar names don't slip through.
    //   3. `redeemId` must be a bigint; we cap at 2^53 so we don't
    //      silently lose precision converting to a decimal string.
    //   4. `dolBurned` must be a non-negative bigint below our sane
    //      supply ceiling (1 B Dol × 10^6 decimals). Larger values
    //      are either buggy or adversarial and we drop them.
    //   5. Final requestId string must be plain decimal — the exact
    //      shape the downstream claim handler regex-validates.
    //
    // Failure mode on any check: the log is silently ignored. If no
    // log passes all checks, we surface the existing warning and
    // leave the pending list untouched — never worse than before.

    const SAFE_INT_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);
    // 1 B Dol × 10^6 decimals = 10^15 wei — above any conceivable
    // on-chain redeem amount for Phase 1 demo traffic, below the
    // precision loss threshold of Number(). Written as a literal
    // because our tsconfig targets es2017 which doesn't support
    // the bigint `**` operator.
    const MAX_SANE_SHARES = BigInt("1000000000000000");
    const seniorAddressLower = senior.address.toLowerCase();

    let decodedId: string | null = null;
    let decodedShares: number | null = null;

    for (const log of requestReceipt.logs) {
      // Guard 1: only logs from our contract. Spoof-resistant.
      if (
        typeof log.address !== "string" ||
        log.address.toLowerCase() !== seniorAddressLower
      ) {
        continue;
      }
      try {
        const decoded = decodeEventLog({
          abi: senior.abi as Abi,
          data: log.data,
          topics: log.topics,
        });
        // Guard 2: event name allowlist
        if (decoded.eventName !== "RedeemRequested") continue;

        const args = decoded.args as unknown as Record<string, unknown>;
        const rawId = args.redeemId;
        const rawShares = args.dolBurned;

        // Guard 3: redeemId must be a bigint within Number range.
        // We store it as a decimal string; BigInt → String() preserves
        // the exact value regardless of size, but claimRedeem later
        // passes the string through a `/^\d+$/` regex so we keep it
        // under MAX_SAFE_INTEGER to stay consistent with the display
        // code paths that still use Number().
        if (typeof rawId !== "bigint") continue;
        if (rawId < BigInt(0) || rawId > SAFE_INT_BIGINT) continue;
        const idStr = rawId.toString();
        if (!/^\d+$/.test(idStr)) continue;

        // Guard 4: dolBurned must be a non-negative bigint under the
        // sane supply ceiling. Anything larger is adversarial.
        if (typeof rawShares !== "bigint") continue;
        if (rawShares < BigInt(0) || rawShares > MAX_SANE_SHARES) continue;

        decodedId = idStr;
        decodedShares = Number(rawShares) / 10 ** SHARE_DECIMALS;
        if (!Number.isFinite(decodedShares) || decodedShares < 0) {
          decodedShares = 0;
        }
        break;
      } catch {
        // decoder threw — malformed log, not a RedeemRequested from
        // us, or an ABI mismatch. Silently skip.
      }
    }

    if (decodedId) {
      const newReq: PendingRedeem = {
        requestId: decodedId,
        shares: decodedShares ?? 0,
        requestedAt: Date.now(),
      };
      setPending((prev) => {
        // dedup by requestId — re-running effect shouldn't double-store
        if (prev.some((r) => r.requestId === decodedId)) return prev;
        const next = [...prev, newReq];
        savePending(userAddress, next);
        return next;
      });
    } else {
      // Defensive log — if this fires, the decode failed silently.
      // eslint-disable-next-line no-console
      console.warn(
        "[useDolWithdraw] Redeem tx confirmed but RedeemRequested event " +
          "could not be decoded or all candidate logs failed the guard " +
          "checks. Check receipt logs, the Dol contract address, and ABI " +
          "event names.",
        requestReceipt,
      );
    }
  }, [
    isRequestConfirmed,
    requestReceipt,
    userAddress,
    senior.abi,
    senior.address,
  ]);

  // Broadcast on every request-redeem confirmation so the global read
  // hooks (useDolBalance on the homepage, LiveVaultTicker, etc.) refetch
  // in the same frame — no stale "my Dol is still there" flash.
  useEffect(() => {
    if (isRequestConfirmed) {
      emitDolTxConfirmed("request-redeem", requestHash);
    }
  }, [isRequestConfirmed, requestHash]);

  // ── Claim redeem (pBondSenior.claimRedeem) ─────────────────────────
  const [claimingId, setClaimingId] = useState<string | null>(null);

  const {
    writeContract: doClaim,
    data: claimHash,
    error: claimError,
    isPending: isClaimPending,
    reset: resetClaim,
  } = useWriteContract();

  const { isLoading: isClaimConfirming, isSuccess: isClaimConfirmed } =
    useWaitForTransactionReceipt({
      hash: claimHash,
      chainId: TARGET_CHAIN_ID,
    });

  useEffect(() => {
    if (!isClaimConfirmed || !claimingId || !userAddress) return;
    // Success: remove the claimed request from the pending list.
    // Explicitly only runs on confirmed success — a failed claim tx
    // leaves the pending entry untouched so the user can see it was
    // NOT claimed and tap the original request to investigate.
    setPending((prev) => {
      const next = prev.filter((r) => r.requestId !== claimingId);
      savePending(userAddress, next);
      return next;
    });
    setClaimingId(null);
  }, [isClaimConfirmed, claimingId, userAddress]);

  // On claim error (wallet rejection, gas revert, already-claimed),
  // clear the `claimingId` marker so the UI's spinner state ends and
  // the user can retry. Crucially we do NOT clear the pending entry
  // itself — it stays in the list so the user can investigate via
  // block explorer what actually happened.
  useEffect(() => {
    if (claimError) {
      setClaimingId(null);
    }
  }, [claimError]);

  useEffect(() => {
    if (isClaimConfirmed) {
      emitDolTxConfirmed("claim-redeem", claimHash);
    }
  }, [isClaimConfirmed, claimHash]);

  // ── Instant redeem (pBondSenior.instantRedeem, Plan A) ────────────
  const {
    writeContract: doInstant,
    data: instantHash,
    error: instantError,
    isPending: isInstantPending,
    reset: resetInstant,
  } = useWriteContract();

  const { isLoading: isInstantConfirming, isSuccess: isInstantConfirmed } =
    useWaitForTransactionReceipt({
      hash: instantHash,
      chainId: TARGET_CHAIN_ID,
    });

  useEffect(() => {
    if (isInstantConfirmed) {
      emitDolTxConfirmed("instant-redeem", instantHash);
    }
  }, [isInstantConfirmed, instantHash]);

  // ── Actions ───────────────────────────────────────────────────────
  const requestRedeem = useCallback(
    (shares: bigint) => {
      if (!isValidAddress(senior.address)) {
        // Refuse to sign a tx against a malformed / injected address
        // eslint-disable-next-line no-console
        console.error(
          "[useDolWithdraw] refusing requestRedeem: invalid senior address",
          senior.address,
        );
        return;
      }
      if (shares <= BigInt(0)) return;
      doRequest({
        address: senior.address,
        abi: senior.abi as Abi,
        functionName: "redeem",
        args: [shares],
        chainId: TARGET_CHAIN_ID,
      });
    },
    [senior.address, senior.abi, doRequest],
  );

  const instantRedeem = useCallback(
    (shares: bigint) => {
      if (!isValidAddress(senior.address)) {
        // eslint-disable-next-line no-console
        console.error(
          "[useDolWithdraw] refusing instantRedeem: invalid senior address",
          senior.address,
        );
        return;
      }
      if (shares <= BigInt(0)) return;
      doInstant({
        address: senior.address,
        abi: senior.abi as Abi,
        functionName: "instantRedeem",
        args: [shares],
        chainId: TARGET_CHAIN_ID,
      });
    },
    [senior.address, senior.abi, doInstant],
  );

  const claimRedeem = useCallback(
    (requestId: string) => {
      if (!isValidAddress(senior.address)) {
        // eslint-disable-next-line no-console
        console.error(
          "[useDolWithdraw] refusing claimRedeem: invalid senior address",
          senior.address,
        );
        return;
      }
      // requestId must be a plain decimal string — prevent injected
      // non-numeric or exotic values from making it into BigInt()
      if (!/^\d+$/.test(requestId)) {
        // eslint-disable-next-line no-console
        console.error(
          "[useDolWithdraw] refusing claimRedeem: invalid requestId",
          requestId,
        );
        return;
      }
      setClaimingId(requestId);
      doClaim({
        address: senior.address,
        abi: senior.abi as Abi,
        functionName: "claimRedeem",
        args: [BigInt(requestId)],
        chainId: TARGET_CHAIN_ID,
      });
    },
    [senior.address, senior.abi, doClaim],
  );

  const isClaimable = useCallback(
    (req: PendingRedeem): boolean => Date.now() - req.requestedAt >= cooldownMs,
    [cooldownMs],
  );

  const cooldownRemaining = useCallback(
    (req: PendingRedeem): number =>
      Math.max(0, cooldownMs - (Date.now() - req.requestedAt)),
    [cooldownMs],
  );

  const reset = useCallback(() => {
    resetRequest();
    resetClaim();
    resetInstant();
    setClaimingId(null);
  }, [resetRequest, resetClaim, resetInstant]);

  return {
    // State
    isConnected,
    cooldownMs, // on-chain 1800s, or 30-min fallback
    // Request redeem (scheduled)
    isRequesting: isRequestPending || isRequestConfirming,
    isRequestConfirmed,
    requestError,
    requestRedeem,
    requestHash,
    // Claim redeem
    isClaiming: isClaimPending || isClaimConfirming,
    isClaimConfirmed,
    claimingId,
    claimError,
    claimRedeem,
    claimHash,
    isClaimable,
    cooldownRemaining,
    // Instant redeem (Plan A)
    isInstantPending: isInstantPending || isInstantConfirming,
    isInstantConfirmed,
    instantError,
    instantRedeem,
    instantHash,
    // Pending
    pending,
    // Util
    reset,
  };
}
