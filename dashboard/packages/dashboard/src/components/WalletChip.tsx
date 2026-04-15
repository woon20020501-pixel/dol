"use client";

import { useState, useEffect, useRef } from "react";
import Link from "next/link";
import { useAccount, useReadContract } from "wagmi";
import { baseSepolia } from "wagmi/chains";
import { useWallets, usePrivy } from "@privy-io/react-auth";
import { toast } from "sonner";
import {
  Copy,
  Check,
  LogOut,
  RefreshCw,
  PlusCircle,
  BellDot,
} from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { type Abi } from "viem";
import { getPBondConfig } from "@/lib/pbond";
import { getVaultConfig } from "@/lib/vault";
import { log } from "@/lib/logger";
import {
  parseLocalStorageArray,
  isValidPendingRedeem,
} from "@/lib/guards";

const TARGET_CHAIN_ID = baseSepolia.id;
const DOL_DECIMALS = 6;
const DOL_SYMBOL = "DOL";

/**
 * Header wallet chip with:
 *  - Network status dot (green = right chain, orange = wrong, red = disconnected)
 *  - Truncated address (click to open menu)
 *  - Dropdown menu: Copy address, Switch network, Disconnect
 *
 * Zero layout shift: same width whether opened or closed.
 */
export default function WalletChip() {
  const { ready, authenticated, logout } = usePrivy();
  const { address } = useAccount();
  const { wallets } = useWallets();
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Read wallet chain (Privy is the source of truth, wagmi follows)
  const activeWallet = wallets[0];
  const walletChainId = activeWallet
    ? Number(activeWallet.chainId.split(":")[1] ?? activeWallet.chainId)
    : null;
  const isWrongChain = authenticated && walletChainId !== null && walletChainId !== TARGET_CHAIN_ID;

  // ── Claimable withdraw detection ─────────────────────────────────
  //
  // A scheduled redeem becomes claimable once `cooldownSeconds` have
  // passed since it was requested. The truthful source of pending
  // redeems is the same localStorage key that `useDolWithdraw` writes
  // to (`dol_pending_redeems_<addr>`); we read it directly here so
  // WalletChip doesn't pull in the withdraw hook's recovery scanner
  // (which would fire 21+ RPC calls per page mount).
  //
  // When a claimable redeem exists we surface a tiny green pulse dot
  // next to the address and add a "Claim ready" menu item — the user
  // can see the badge from ANY route, not just /my-dol.
  //
  const vault = getVaultConfig();
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
      : 30 * 60 * 1000;

  const [claimableCount, setClaimableCount] = useState<number>(0);
  useEffect(() => {
    if (!address) {
      setClaimableCount(0);
      return;
    }
    const key = `dol_pending_redeems_${address}`;
    const check = () => {
      try {
        const raw = localStorage.getItem(key);
        const parsed = parseLocalStorageArray(raw, isValidPendingRedeem) as {
          requestId: string;
          shares: number;
          requestedAt: number;
        }[];
        const now = Date.now();
        const ready = parsed.filter((r) => now - r.requestedAt >= cooldownMs);
        setClaimableCount(ready.length);
      } catch {
        setClaimableCount(0);
      }
    };
    check();
    // Re-check every 15 s so the badge appears automatically once the
    // cooldown elapses, without requiring the user to refresh.
    const interval = setInterval(check, 15_000);
    // Also re-check when the tab regains focus — common pattern for
    // users who opened /my-dol in another tab.
    const onFocus = () => check();
    window.addEventListener("focus", onFocus);
    // And react to explicit storage events (other tabs writing to it).
    const onStorage = (e: StorageEvent) => {
      if (e.key === key) check();
    };
    window.addEventListener("storage", onStorage);
    return () => {
      clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("storage", onStorage);
    };
  }, [address, cooldownMs]);

  // Click outside to close
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onClick);
    return () => window.removeEventListener("mousedown", onClick);
  }, [open]);

  // Close on escape
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!ready || !authenticated || !address) return null;

  const copyAddress = async () => {
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      toast.success("Address copied", { duration: 1500 });
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Couldn't copy address", {
        description: "Your browser blocked clipboard access. Try manually.",
      });
    }
  };

  const switchNetwork = async () => {
    if (!activeWallet) return;
    try {
      await activeWallet.switchChain(TARGET_CHAIN_ID);
      toast.success("Network switched to Base Sepolia");
      setOpen(false);
    } catch {
      toast.error("Couldn't switch network", {
        description:
          "Open your wallet and switch to Base Sepolia manually, then retry.",
      });
    }
  };

  const addToWallet = async () => {
    if (!activeWallet) return;
    try {
      const provider = await activeWallet.getEthereumProvider();
      const { senior } = getPBondConfig();
      // Absolute URL required — MetaMask rejects relative paths.
      // Skip the image on localhost since MetaMask can't fetch from
      // loopback in some setups; fall back to symbol-only registration.
      const origin =
        typeof window !== "undefined" ? window.location.origin : "";
      const isLocal = /^https?:\/\/(localhost|127\.|\[::1\])/.test(origin);
      // 256x256 icon — MetaMask rejects or downscales the 2048px hero
      const image = isLocal ? undefined : `${origin}/images/dol-icon.png`;

      const wasAdded = (await provider.request({
        method: "wallet_watchAsset",
        params: {
          type: "ERC20",
          options: {
            address: senior.address,
            symbol: DOL_SYMBOL,
            decimals: DOL_DECIMALS,
            ...(image ? { image } : {}),
          },
        },
        // viem/Privy typings want params as an array for most methods,
        // but wallet_watchAsset takes a single object per EIP-747.
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any)) as boolean;

      if (wasAdded) {
        toast.success("Dol added to your wallet.");
      } else {
        toast("Not added.", { duration: 1500 });
      }
      setOpen(false);
    } catch (e) {
      log.warn("[WalletChip] wallet_watchAsset failed:", e);
      toast.error("Your wallet doesn't support this. Add Dol manually.");
    }
  };

  const doLogout = () => {
    setOpen(false);
    logout();
  };

  const truncated = `${address.slice(0, 6)}\u2026${address.slice(-4)}`;

  // Status dot colors
  const dotColor = isWrongChain
    ? "bg-amber-400"
    : "bg-emerald-400";
  const dotPulse = isWrongChain ? "animate-pulse" : "";

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label={
          claimableCount > 0
            ? `Wallet menu — ${claimableCount} withdraw ready to claim`
            : "Wallet menu"
        }
        aria-expanded={open}
        className="relative flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.04] px-3 py-1.5 hover:border-white/20 hover:bg-white/[0.08] transition-colors"
      >
        <span
          className={`inline-block h-1.5 w-1.5 rounded-full ${dotColor} ${dotPulse}`}
          aria-hidden
        />
        <span className="font-mono text-[12px] text-white/80">
          {truncated}
        </span>
        {/* Claimable badge — pulsing green dot when one or more
            scheduled withdraws are past their cooldown. Sits on the
            top-right of the chip so the user notices it from any
            page, not just /my-dol. */}
        {claimableCount > 0 && (
          <span
            className="absolute -right-0.5 -top-0.5 flex h-2.5 w-2.5"
            aria-hidden
          >
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60" />
            <span className="relative inline-flex h-2.5 w-2.5 rounded-full bg-emerald-400 ring-2 ring-[#0a0a0a]" />
          </span>
        )}
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: -6, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -6, scale: 0.97 }}
            transition={{ duration: 0.15, ease: [0.05, 0.7, 0.1, 1.0] }}
            className="absolute right-0 top-full mt-2 w-64 rounded-2xl border border-white/10 bg-[#0a0a0a] p-1 shadow-2xl backdrop-blur-xl"
            style={{ transformOrigin: "top right" }}
            role="menu"
          >
            {/* Claim-ready reminder — the highest-priority action when
                present, so it sits at the top of the menu and visually
                pops with a subtle emerald tint. Navigates to /my-dol
                where the actual Claim button lives. */}
            {claimableCount > 0 && (
              <>
                <Link
                  href="/my-dol"
                  onClick={() => setOpen(false)}
                  role="menuitem"
                  className="flex w-full items-center gap-3 rounded-xl bg-emerald-500/[0.08] px-3 py-3 text-left text-[13px] font-semibold text-emerald-300 hover:bg-emerald-500/[0.12] transition-colors"
                >
                  <BellDot className="h-4 w-4" />
                  <span className="flex-1">
                    Your Dol is ready
                    <span className="ml-2 text-[11px] font-normal text-emerald-300/60">
                      Tap to claim
                    </span>
                  </span>
                </Link>
                <div className="my-1 border-t border-white/5" />
              </>
            )}
            <button
              type="button"
              onClick={copyAddress}
              role="menuitem"
              className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-[13px] text-white/80 hover:bg-white/[0.06] transition-colors"
            >
              {copied ? (
                <Check className="h-4 w-4 text-emerald-400" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
              {copied ? "Copied" : "Copy address"}
            </button>

            {isWrongChain && (
              <button
                type="button"
                onClick={switchNetwork}
                role="menuitem"
                className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-[13px] text-amber-300 hover:bg-white/[0.06] transition-colors"
              >
                <RefreshCw className="h-4 w-4" />
                Switch to Base Sepolia
              </button>
            )}

            {/* Add Dol to wallet — shows the actual contract address
                as a subtitle inside the menu item, so a user who is
                about to add a token to their wallet sees WHICH token
                before they click. Passive defense against a malicious
                browser extension that swaps the address we'd hand to
                wallet_watchAsset: the user has a visible reference to
                compare against what MetaMask subsequently shows. No
                extra click, no modal, zero added friction. */}
            <button
              type="button"
              onClick={addToWallet}
              role="menuitem"
              className="flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left text-[13px] text-white/80 hover:bg-white/[0.06] transition-colors"
            >
              <PlusCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span className="flex-1">
                Add Dol to wallet
                <span className="mt-0.5 block font-mono text-[10px] text-white/35">
                  {(() => {
                    const { senior } = getPBondConfig();
                    return `${senior.address.slice(0, 6)}…${senior.address.slice(-4)} · Base Sepolia`;
                  })()}
                </span>
              </span>
            </button>

            <div className="my-1 border-t border-white/5" />

            <button
              type="button"
              onClick={doLogout}
              role="menuitem"
              className="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-[13px] text-white/60 hover:bg-white/[0.06] hover:text-white/90 transition-colors"
            >
              <LogOut className="h-4 w-4" />
              Disconnect
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
