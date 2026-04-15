"use client";

import { useState, useCallback, useEffect } from "react";
import {
  useAccount,
  useWriteContract,
  useWaitForTransactionReceipt,
  useSwitchChain,
  useChainId,
} from "wagmi";
import { parseUnits, decodeEventLog } from "viem";
import { getVaultConfig, VAULT_ABI } from "@/lib/vault";

const SHARE_DECIMALS = 6;
const COOLDOWN_MS = 24 * 60 * 60 * 1000; // 24 hours
const EXPECTED_CHAIN_ID = Number(process.env.NEXT_PUBLIC_CHAIN_ID || "84532");
const STORAGE_KEY = "pacifica_pending_withdraws";

export type PendingRequest = {
  requestId: string;
  shares: number;
  requestedAt: number; // unix ms
};

/** Load pending requests from localStorage, scoped by user address */
function loadPendingRequests(userAddress: string): PendingRequest[] {
  try {
    const raw = localStorage.getItem(`${STORAGE_KEY}_${userAddress}`);
    return raw ? (JSON.parse(raw) as PendingRequest[]) : [];
  } catch {
    return [];
  }
}

/** Save pending requests to localStorage */
function savePendingRequests(
  userAddress: string,
  requests: PendingRequest[],
) {
  try {
    localStorage.setItem(
      `${STORAGE_KEY}_${userAddress}`,
      JSON.stringify(requests),
    );
  } catch {
    // localStorage might be unavailable
  }
}

export function useWithdraw() {
  const vaultConfig = getVaultConfig();
  const { address: userAddress, isConnected } = useAccount();
  const chainId = useChainId();
  const { switchChain } = useSwitchChain();

  const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";
  const isWrongNetwork = isConnected && chainId !== EXPECTED_CHAIN_ID;

  // ── Pending requests (localStorage-backed) ─────────────────────

  const [pendingRequests, setPendingRequests] = useState<PendingRequest[]>([]);

  // Load from localStorage on mount / address change
  useEffect(() => {
    if (userAddress) {
      setPendingRequests(loadPendingRequests(userAddress));
    } else {
      setPendingRequests([]);
    }
  }, [userAddress]);

  // ── Request withdraw write ─────────────────────────────────────

  const {
    writeContract: doRequest,
    data: requestHash,
    error: requestError,
    isPending: isRequestPending,
    reset: resetRequest,
  } = useWriteContract();

  const {
    data: requestReceipt,
    isLoading: isRequestConfirming,
    isSuccess: isRequestConfirmed,
  } = useWaitForTransactionReceipt({ hash: requestHash });

  // Parse WithdrawRequested event from receipt and track it
  useEffect(() => {
    if (!isRequestConfirmed || !requestReceipt || !userAddress) return;

    for (const log of requestReceipt.logs) {
      try {
        const decoded = decodeEventLog({
          abi: VAULT_ABI,
          data: log.data,
          topics: log.topics,
        });
        if (decoded.eventName === "WithdrawRequested") {
          const args = decoded.args as { id: bigint; shares: bigint };
          const newReq: PendingRequest = {
            requestId: args.id.toString(),
            shares: Number(
              (Number(args.shares) / 10 ** SHARE_DECIMALS).toFixed(
                SHARE_DECIMALS,
              ),
            ),
            requestedAt: Date.now(),
          };
          setPendingRequests((prev) => {
            const next = [...prev, newReq];
            savePendingRequests(userAddress, next);
            return next;
          });
          break;
        }
      } catch {
        // not our event, skip
      }
    }
  }, [isRequestConfirmed, requestReceipt, userAddress]);

  // ── Claim withdraw write ───────────────────────────────────────

  const [claimingId, setClaimingId] = useState<string | null>(null);

  const {
    writeContract: doClaim,
    data: claimHash,
    error: claimError,
    isPending: isClaimPending,
    reset: resetClaim,
  } = useWriteContract();

  const {
    isLoading: isClaimConfirming,
    isSuccess: isClaimConfirmed,
  } = useWaitForTransactionReceipt({ hash: claimHash });

  // Remove claimed request from pending list
  useEffect(() => {
    if (!isClaimConfirmed || !claimingId || !userAddress) return;

    setPendingRequests((prev) => {
      const next = prev.filter((r) => r.requestId !== claimingId);
      savePendingRequests(userAddress, next);
      return next;
    });
    setClaimingId(null);
  }, [isClaimConfirmed, claimingId, userAddress]);

  // ── Actions ─────────────────────────────────────────────────────

  const requestWithdraw = useCallback(
    (shares: number) => {
      if (!vaultConfig) return;
      doRequest({
        address: vaultConfig.address,
        abi: VAULT_ABI,
        functionName: "requestWithdraw",
        args: [parseUnits(shares.toString(), SHARE_DECIMALS)],
      });
    },
    [vaultConfig, doRequest],
  );

  const claimWithdraw = useCallback(
    (requestId: string) => {
      if (!vaultConfig) return;
      setClaimingId(requestId);
      doClaim({
        address: vaultConfig.address,
        abi: VAULT_ABI,
        functionName: "claimWithdraw",
        args: [BigInt(requestId)],
      });
    },
    [vaultConfig, doClaim],
  );

  const isClaimable = useCallback((req: PendingRequest): boolean => {
    return Date.now() - req.requestedAt >= COOLDOWN_MS;
  }, []);

  const cooldownRemaining = useCallback((req: PendingRequest): number => {
    const elapsed = Date.now() - req.requestedAt;
    return Math.max(0, COOLDOWN_MS - elapsed);
  }, []);

  const reset = useCallback(() => {
    resetRequest();
    resetClaim();
    setClaimingId(null);
  }, [resetRequest, resetClaim]);

  const switchNetwork = useCallback(() => {
    switchChain({ chainId: EXPECTED_CHAIN_ID });
  }, [switchChain]);

  // ── Derived state ──────────────────────────────────────────────

  const isRequesting = isRequestPending || isRequestConfirming;
  const isClaiming = isClaimPending || isClaimConfirming;

  const requestErrorMsg = requestError
    ? (requestError as Error).message?.split("\n")[0] ?? "Request failed"
    : null;
  const claimErrorMsg = claimError
    ? (claimError as Error).message?.split("\n")[0] ?? "Claim failed"
    : null;

  return {
    // State
    deployed: !!vaultConfig,
    isDemoMode,
    isWrongNetwork,
    isConnected,
    // Request
    isRequesting,
    isRequestConfirmed,
    requestError: requestErrorMsg,
    requestWithdraw,
    // Claim
    isClaiming,
    isClaimConfirmed,
    claimingId,
    claimError: claimErrorMsg,
    claimWithdraw,
    isClaimable,
    cooldownRemaining,
    // Pending
    pendingRequests,
    // Util
    reset,
    switchNetwork,
  };
}
