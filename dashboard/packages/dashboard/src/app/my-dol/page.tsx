"use client";

import Link from "next/link";
import { useState, useRef, useEffect, Suspense } from "react";
import { useSearchParams, useRouter } from "next/navigation";
import {
  motion,
  AnimatePresence,
  useMotionValue,
  animate,
  type Transition,
} from "framer-motion";
import { ArrowLeft, Loader2, Clock, CheckCircle } from "lucide-react";
import { toast } from "sonner";
import { useReadContract } from "wagmi";
import { baseSepolia } from "wagmi/chains";
import { getVaultConfig, ERC20_ABI } from "@/lib/vault";
import DolHeroImage from "@/components/DolHeroImage";
import LiveCounter from "@/components/LiveCounter";
import { CashoutSheet } from "@/components/LazyClientComponents";
import WalletChip from "@/components/WalletChip";
import { SiteFooter } from "@/components/SiteFooter";
import { MobileMenu } from "@/components/MobileMenu";
import { usePrivy } from "@privy-io/react-auth";
import { useDolBalance } from "@/hooks/useDolBalance";
import { useDolWithdraw, type PendingRedeem } from "@/hooks/useDolWithdraw";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useTxHistory, type LoggedTx } from "@/hooks/useTxHistory";
import { translateError } from "@/lib/errors";

const APPLE_EASE = [0.05, 0.7, 0.1, 1.0] as const;

const SOFT_SPRING: Transition = {
  type: "spring",
  stiffness: 400,
  damping: 35,
  mass: 1.2,
};

const LANDING_SPRING: Transition = {
  type: "spring",
  stiffness: 600,
  damping: 25,
  mass: 1.5,
};

import { DOL_APY } from "@/lib/constants";
const DOL_RATE = DOL_APY;

type Phase = "empty" | "descending" | "landed" | "active";

const BASESCAN = "https://sepolia.basescan.org";

const SHARE_DECIMALS = 6;

/**
 * Convert a user-facing Dol amount (float) to raw share units (bigint).
 *
 * Guards:
 *   - Non-finite or ≤0 amount → 0n (caller rejects)
 *   - `BigInt(Math.floor(NaN))` would throw RangeError; we filter first
 *   - Clamps to `maxShares` so the user can't request more than they hold,
 *     even if floating-point rounding on the UI side produced a value
 *     that would otherwise over-spend their balance by one wei.
 */
function toShares(
  amount: number,
  maxShares: bigint | null,
): bigint {
  if (!Number.isFinite(amount) || amount <= 0) return BigInt(0);
  const whole = Math.floor(amount * 10 ** SHARE_DECIMALS);
  if (!Number.isFinite(whole) || whole <= 0) return BigInt(0);
  let shares: bigint;
  try {
    shares = BigInt(whole);
  } catch {
    return BigInt(0);
  }
  if (maxShares !== null && shares > maxShares) return maxShares;
  return shares;
}

function MyDolInner() {
  const searchParams = useSearchParams();
  const router = useRouter();
  useKeyboardShortcuts();
  const { ready, authenticated, login } = usePrivy();

  const fresh = searchParams.get("fresh") === "1";

  /* Real on-chain balance via useDolBalance */
  const {
    balanceShares,
    balance: realBalance,
    usdcValue,
    hasBalance,
    isLoading: balanceLoading,
    refetch: refetchBalance,
  } = useDolBalance();

  /* Withdraw hook — request + claim against pBondSenior */
  const wd = useDolWithdraw();

  /* Instant preflight — read vault's USDC buffer and compare against
     the USDC equivalent of the user's Dol balance. If the buffer can't
     cover the full redeem, Instant would revert with InsufficientBalance
     and MetaMask surfaces it as "exceeds max transaction gas limit".
     We grey out the Instant button before the user taps it so they get
     a clean "use Scheduled" fallback instead of the raw gas error.
      */
  const vaultCfg = getVaultConfig();
  const { data: vaultUsdcRaw, refetch: refetchVaultUsdc } = useReadContract({
    address: vaultCfg?.usdcAddress,
    abi: ERC20_ABI,
    functionName: "balanceOf",
    args: vaultCfg?.address ? [vaultCfg.address] : undefined,
    chainId: baseSepolia.id,
    query: { enabled: !!vaultCfg?.usdcAddress && !!vaultCfg?.address },
  });
  const vaultUsdcBalance =
    typeof vaultUsdcRaw === "bigint" ? vaultUsdcRaw : null;
  // usdcValue is the USDC-equivalent of the user's Dol balance (6 decimals).
  // Convert to raw USDC units and compare. If we can't read the buffer
  // yet, fall open (don't block the button pre-emptively).
  //
  // Guard: `BigInt(Math.ceil(NaN))` throws a RangeError and would punt
  // the whole route to app/error.tsx. We explicitly bail on non-finite
  // or negative input and keep the flag false in that case, which lets
  // the user proceed and lets the real revert (if any) get handled by
  // the withdraw error translator instead.
  const userUsdcWei = (() => {
    if (!Number.isFinite(usdcValue) || usdcValue <= 0) return BigInt(0);
    const whole = Math.ceil(usdcValue * 1_000_000);
    if (!Number.isFinite(whole)) return BigInt(0);
    try {
      return BigInt(whole);
    } catch {
      return BigInt(0);
    }
  })();
  const instantBufferShort =
    vaultUsdcBalance !== null &&
    userUsdcWei > BigInt(0) &&
    vaultUsdcBalance < userUsdcWei;

  // Re-poll the buffer after any withdraw/claim tx confirms so the button
  // re-enables automatically once the buffer recovers.
  useEffect(() => {
    if (wd.isInstantConfirmed || wd.isClaimConfirmed || wd.isRequestConfirmed) {
      refetchVaultUsdc();
    }
  }, [
    wd.isInstantConfirmed,
    wd.isClaimConfirmed,
    wd.isRequestConfirmed,
    refetchVaultUsdc,
  ]);

  /* Local tx history (localStorage) */
  const txHistory = useTxHistory();

  /* Phase state */
  const [phase, setPhase] = useState<Phase>("empty");
  const [cashoutOpen, setCashoutOpen] = useState(false);
  const rampBalance = useMotionValue(0);
  const counterRef = useRef<HTMLSpanElement>(null);
  const birthPlayedRef = useRef(false);

  /* Decide phase based on real balance + fresh flag */
  useEffect(() => {
    if (!authenticated) {
      setPhase("empty");
      return;
    }
    if (balanceLoading) return;

    if (realBalance > 0) {
      if (fresh && !birthPlayedRef.current) {
        // Play birth sequence once, then clean URL
        birthPlayedRef.current = true;
        triggerBirthSequence(realBalance);
      } else if (phase !== "active") {
        // Go straight to active
        setPhase("active");
      }
    } else {
      setPhase("empty");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authenticated, balanceLoading, realBalance, fresh]);

  /* Birth sequence — descends, lands, ramps, activates */
  const triggerBirthSequence = (targetBalance: number) => {
    setPhase("descending");

    setTimeout(() => setPhase("landed"), 1100);

    setTimeout(() => {
      animate(rampBalance, targetBalance, {
        duration: 1.4,
        ease: [0.16, 1, 0.3, 1],
        onUpdate: (v) => {
          if (counterRef.current) counterRef.current.innerText = v.toFixed(6);
        },
        onComplete: () => {
          setTimeout(() => {
            setPhase("active");
            // Clean URL
            router.replace("/my-dol", { scroll: false });
          }, 200);
        },
      });
    }, 1400);
  };

  /* Empty state action — go to deposit */
  const hasRealBalance = hasBalance;

  /* Toast on withdraw tx state transitions */
  useEffect(() => {
    if (wd.isRequesting && !wd.isRequestConfirmed) {
      toast("Sending your request...", { id: "redeem-req" });
    }
  }, [wd.isRequesting, wd.isRequestConfirmed]);

  useEffect(() => {
    if (wd.isRequestConfirmed && wd.requestHash) {
      toast.success("Scheduled. We'll let you know when it's ready.", {
        id: "redeem-req",
        description: "It's in the queue.",
        action: {
          label: "View tx",
          onClick: () =>
            window.open(`${BASESCAN}/tx/${wd.requestHash}`, "_blank", "noopener,noreferrer"),
        },
      });
      // Defensive: never let a bad tx history write bubble up to the
      // route error boundary. Amount might momentarily be non-finite
      // during a refetch transition — clamp to 0 in that case.
      try {
        txHistory.record({
          hash: wd.requestHash,
          type: "redeem-scheduled",
          amount: Number.isFinite(usdcValue) ? usdcValue : 0,
        });
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn("[my-dol] txHistory.record failed:", e);
      }
      refetchBalance();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.isRequestConfirmed, wd.requestHash, refetchBalance]);

  useEffect(() => {
    if (wd.requestError) {
      const e = translateError(wd.requestError);
      if (e.category === "user_rejected") {
        toast(e.title, { id: "redeem-req" });
      } else {
        // Reverted on-chain: attach a "View tx" action so the user
        // can inspect the revert reason directly on basescan. User-
        // rejection path deliberately skipped — no tx to link to.
        const action =
          wd.isReverted && wd.revertedHash
            ? {
                label: "View tx",
                onClick: () =>
                  window.open(
                    `${BASESCAN}/tx/${wd.revertedHash}`,
                    "_blank",
                    "noopener,noreferrer",
                  ),
              }
            : undefined;
        toast.error(e.title, {
          id: "redeem-req",
          description: e.description,
          action,
        });
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.requestError]);

  useEffect(() => {
    if (wd.isClaiming && !wd.isClaimConfirmed) {
      toast("Claiming...", { id: "redeem-claim" });
    }
  }, [wd.isClaiming, wd.isClaimConfirmed]);

  useEffect(() => {
    if (wd.isClaimConfirmed && wd.claimHash) {
      toast.success("Done. It's back in your wallet.", {
        id: "redeem-claim",
        action: {
          label: "View tx",
          onClick: () =>
            window.open(`${BASESCAN}/tx/${wd.claimHash}`, "_blank", "noopener,noreferrer"),
        },
      });
      try {
        txHistory.record({
          hash: wd.claimHash,
          type: "claim",
          amount: Number.isFinite(usdcValue) ? usdcValue : 0,
        });
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn("[my-dol] txHistory.record failed:", e);
      }
      refetchBalance();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.isClaimConfirmed, wd.claimHash, refetchBalance]);

  useEffect(() => {
    if (wd.claimError) {
      const e = translateError(wd.claimError);
      const action =
        wd.isReverted && wd.revertedHash
          ? {
              label: "View tx",
              onClick: () =>
                window.open(
                  `${BASESCAN}/tx/${wd.revertedHash}`,
                  "_blank",
                  "noopener,noreferrer",
                ),
            }
          : undefined;
      toast.error(e.title, {
        id: "redeem-claim",
        description: e.description,
        action,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.claimError]);

  /* Instant redeem toasts + InsufficientLiquidity fallback */
  useEffect(() => {
    if (wd.isInstantPending && !wd.isInstantConfirmed) {
      toast("Sending...", { id: "redeem-instant" });
    }
  }, [wd.isInstantPending, wd.isInstantConfirmed]);

  useEffect(() => {
    if (wd.isInstantConfirmed && wd.instantHash) {
      toast.success("Sent. It's in your wallet.", {
        id: "redeem-instant",
        action: {
          label: "View tx",
          onClick: () =>
            window.open(`${BASESCAN}/tx/${wd.instantHash}`, "_blank", "noopener,noreferrer"),
        },
      });
      try {
        txHistory.record({
          hash: wd.instantHash,
          type: "redeem-instant",
          amount: Number.isFinite(usdcValue) ? usdcValue : 0,
        });
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn("[my-dol] txHistory.record failed:", e);
      }
      refetchBalance();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.isInstantConfirmed, wd.instantHash, refetchBalance]);

  useEffect(() => {
    if (!wd.instantError) return;
    const msg = String((wd.instantError as Error).message || "").toLowerCase();

    // InsufficientLiquidity — we no longer auto-fallback to Scheduled
    // because the fallback would have had to guess the user's intended
    // amount (they now pick it explicitly in CashoutSheet). Instead,
    // surface a gentle caption and let them pick Scheduled themselves.
    // The preflight on `instantBufferShort` should catch this case
    // before it ever reaches the error path, so landing here means
    // either a race with the buffer read or a different contract
    // revert worth actually showing.
    if (
      msg.includes("insufficientliquidity") ||
      msg.includes("insufficient liquidity")
    ) {
      toast("Instant unavailable right now — try Scheduled.", {
        id: "redeem-instant",
      });
      return;
    }

    // Otherwise use standard error translator
    const e = translateError(wd.instantError);
    if (e.category === "user_rejected") {
      toast(e.title, { id: "redeem-instant" });
    } else {
      const action =
        wd.isReverted && wd.revertedHash
          ? {
              label: "View tx",
              onClick: () =>
                window.open(
                  `${BASESCAN}/tx/${wd.revertedHash}`,
                  "_blank",
                  "noopener,noreferrer",
                ),
            }
          : undefined;
      toast.error(e.title, {
        id: "redeem-instant",
        description: e.description,
        action,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wd.instantError]);

  return (
    <main
      className="relative min-h-screen bg-black text-white overflow-hidden"
      style={{
        background: "radial-gradient(ellipse at top, #0a0a0d 0%, #000000 70%)",
      }}
    >
      {/* Header */}
      <header className="fixed top-0 left-0 right-0 z-50 bg-black/40 backdrop-blur-xl border-b border-white/[0.06]">
        <div className="mx-auto flex h-[56px] max-w-[1080px] items-center justify-between px-5 sm:px-6">
          <Link
            href="/"
            className="text-[20px] font-semibold tracking-[-0.02em] transition-opacity hover:opacity-80"
            aria-label="Dol home"
          >
            Dol
          </Link>
          <nav className="flex items-center gap-5 sm:gap-6" aria-label="Primary">
            <Link
              href="/"
              className="hidden text-[13px] text-white/40 transition-colors hover:text-white sm:inline-flex sm:items-center"
            >
              <ArrowLeft className="mr-1 h-3.5 w-3.5" />
              Home
            </Link>
            <Link
              href="/docs"
              className="hidden text-[13px] text-white/70 transition-colors hover:text-white sm:block"
            >
              Docs
            </Link>
            <Link
              href="/faq"
              className="hidden text-[13px] text-white/70 transition-colors hover:text-white sm:block"
            >
              FAQ
            </Link>
            {!authenticated && ready && (
              <button
                onClick={login}
                className="rounded-full bg-white px-4 py-1.5 text-[13px] font-medium text-black transition-colors hover:bg-white/90 sm:px-5 sm:py-2"
              >
                Connect
              </button>
            )}
            <WalletChip />
            <MobileMenu
              links={[
                { href: "/", label: "Home" },
                { href: "/docs", label: "Docs" },
                { href: "/faq", label: "FAQ" },
              ]}
            />
          </nav>
        </div>
      </header>

      {/* TOP 60% — Apple Showcase
          `isolation: isolate` forces a new stacking context at the section
          level so z-index on the headline / Dol children is honoured even
          when framer-motion's opacity+filter create intermediate contexts.
          `gap-20` (80px) gives a physical-space safety margin on top of the
          z-index — DolHeroImage's outer bloom is `size * 1.6` = 384px for
          a 240px base, extending ~72px above the Dol wrapper; gap-20 keeps
          the bloom's top edge ≥ 8px below the headline's bottom even
          without stacking help. Belt AND suspenders. */}
      <section
        className="relative flex min-h-[60vh] flex-col items-center justify-center gap-16 px-6 pb-8 pt-24 sm:gap-20"
        style={{ isolation: "isolate" }}
      >
        <div
          className="relative z-20 flex min-h-[100px] w-full items-end justify-center sm:min-h-[140px] md:min-h-[170px]"
          style={{ isolation: "isolate" }}
        >
          <AnimatePresence>
            {(phase === "landed" || phase === "active") && (
              <motion.h1
                initial={{ opacity: 0, y: 12, filter: "blur(8px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                exit={{ opacity: 0 }}
                transition={{ duration: 1.0, ease: APPLE_EASE }}
                className="w-full text-center text-[32px] sm:text-[48px] md:text-[60px] font-bold leading-[1.05]"
                style={{
                  letterSpacing: "-0.04em",
                  backgroundImage:
                    "linear-gradient(120deg, #fafafa 0%, #a5b4fc 18%, #f0abfc 35%, #fef3c7 52%, #a5f3fc 70%, #e2e8f0 100%)",
                  WebkitBackgroundClip: "text",
                  backgroundClip: "text",
                  color: "transparent",
                  backgroundSize: "200% 200%",
                  animation: "shimmerGradient 8s ease-in-out infinite",
                }}
              >
                Up to 7.5% a year.
                <br />
                A dollar that grows itself.
              </motion.h1>
            )}
          </AnimatePresence>
        </div>

        <div className="relative z-0 flex w-full items-center justify-center">
          {/* Dol present: descending, landed, or active */}
          {phase !== "empty" && (
            <motion.div
              initial={{ y: -520, opacity: 0, scale: 0.9 }}
              animate={
                phase === "descending"
                  ? { y: 0, opacity: 1, scale: 1 }
                  : {
                      y: 0,
                      opacity: 1,
                      scale: [1, 0.78, 1.08, 0.96, 1.02, 1],
                    }
              }
              transition={
                phase === "descending"
                  ? LANDING_SPRING
                  : { duration: 0.7, ease: [0.2, 0.9, 0.25, 1] }
              }
              className="relative"
            >
              <DolHeroImage size={240} />
            </motion.div>
          )}

          {/* Empty void state */}
          {phase === "empty" && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 1.2, delay: 0.3, ease: APPLE_EASE }}
              className="flex flex-col items-center gap-6 text-white/25 py-16"
            >
              <div className="w-2 h-2 rounded-full bg-white/40 animate-pulse" />
              <p className="text-[13px] tracking-[0.2em] uppercase">
                {!authenticated ? "Connect to see your Dol" : "Your Dol awaits"}
              </p>
            </motion.div>
          )}
        </div>
      </section>

      {/* BOTTOM 40% — Toss Control Center */}
      <motion.section
        className="relative z-10 px-4 pb-8 -mt-24"
        animate={{
          y: phase === "descending" ? 60 : 0,
          opacity: phase === "descending" ? 0.4 : 1,
        }}
        transition={{ duration: 0.5, ease: APPLE_EASE }}
      >
        <div className="mx-auto max-w-[520px]">
          <div
            className="rounded-[32px] p-8 backdrop-blur-2xl border border-white/10"
            style={{
              background:
                "linear-gradient(180deg, rgba(255,255,255,0.06) 0%, rgba(255,255,255,0.03) 100%)",
              boxShadow:
                "0 30px 80px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.05) inset",
            }}
          >
            {/* Balance */}
            <div className="text-center">
              <p className="text-[11px] text-white/40 uppercase tracking-[0.2em]">
                Your balance
              </p>
              <p className="mt-1 text-[10px] text-white/25 tracking-[0.08em]">
                1 Dol = 1 USDC
              </p>
              {/*
                Big balance display — uses `clamp()` so a 6-decimal
                number with a "Dol" suffix always fits on the narrowest
                phone (375 px) while still going huge on desktop. The
                value `clamp(28 px, 9vw, 56 px)` resolves to ~33 px on
                375 px, 54 px at 600 px, then caps at 56 px. The "Dol"
                suffix shares the same responsive scale via Tailwind
                `text-xl sm:text-3xl` so the two never get out of
                proportion with each other. `whitespace-nowrap` plus
                `overflow-x-hidden` on the parent column keeps the
                number on a single line at every breakpoint.
              */}
              <div
                className="mt-3 whitespace-nowrap font-bold text-white leading-[1]"
                style={{
                  fontSize: "clamp(28px, 9vw, 56px)",
                  letterSpacing: "-0.04em",
                }}
              >
                {phase === "active" && hasRealBalance ? (
                  <>
                    <LiveCounter
                      initial={usdcValue}
                      apy={DOL_RATE}
                      decimals={6}
                    />
                    <span className="ml-2 text-xl font-semibold text-white/40 sm:text-3xl">
                      Dol
                    </span>
                  </>
                ) : (
                  <>
                    <span
                      ref={counterRef}
                      className="tabular-nums"
                      style={{ fontVariantNumeric: "tabular-nums" }}
                    >
                      0.000000
                    </span>
                    <span className="ml-2 text-xl font-semibold text-white/40 sm:text-3xl">
                      Dol
                    </span>
                  </>
                )}
              </div>
              {phase === "active" && (
                <motion.p
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.6, ease: APPLE_EASE }}
                  className="mt-2 text-[13px] text-white/45"
                >
                  Growing every second.
                </motion.p>
              )}
              {phase === "empty" && (
                <p className="mt-2 text-[13px] text-white/30">
                  {!authenticated
                    ? "Connect your account to begin."
                    : balanceLoading
                      ? "Checking your balance..."
                      : "Get your first Dol."}
                </p>
              )}
            </div>

            {/* Action buttons */}
            <div className="mt-8 flex flex-col gap-3">
              {!authenticated && ready ? (
                <motion.button
                  whileHover={{ scale: 1.015 }}
                  whileTap={{ scale: 0.96 }}
                  transition={SOFT_SPRING}
                  onClick={login}
                  className="w-full py-4 rounded-full bg-white text-black font-semibold text-[17px]"
                  style={{
                    letterSpacing: "-0.01em",
                    boxShadow:
                      "0 14px 40px rgba(255,255,255,0.18), 0 4px 12px rgba(0,0,0,0.4)",
                  }}
                >
                  Connect to continue
                </motion.button>
              ) : balanceLoading && authenticated ? (
                <div className="w-full py-4 rounded-full bg-white/5 border border-white/10 flex items-center justify-center text-white/40">
                  <Loader2 className="h-5 w-5 animate-spin" />
                </div>
              ) : (
                <>
                  <Link href="/deposit">
                    <motion.div
                      whileHover={{ scale: 1.015 }}
                      whileTap={{ scale: 0.96 }}
                      transition={SOFT_SPRING}
                      className="w-full py-4 rounded-full bg-white text-black font-semibold text-[17px] text-center cursor-pointer"
                      style={{
                        letterSpacing: "-0.01em",
                        boxShadow:
                          "0 14px 40px rgba(255,255,255,0.18), 0 4px 12px rgba(0,0,0,0.4)",
                      }}
                    >
                      {hasRealBalance ? "Grow your Dol" : "Get your first Dol"}
                    </motion.div>
                  </Link>

                  <motion.button
                    whileHover={{ scale: 1.015 }}
                    whileTap={{ scale: 0.96 }}
                    transition={SOFT_SPRING}
                    onClick={() => hasRealBalance && setCashoutOpen(true)}
                    disabled={!hasRealBalance}
                    className="w-full py-4 rounded-full bg-white/[0.06] border border-white/10 text-white/90 font-medium text-[15px] hover:bg-white/[0.1] transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                    style={{ letterSpacing: "-0.01em" }}
                  >
                    Send to my account
                  </motion.button>
                </>
              )}
            </div>

            <p className="mt-5 text-center text-[11px] text-white/25">
              Safe &middot; Auto &middot; Cash out anytime
            </p>
          </div>

          {/* Pending cashout requests — shows after user taps Scheduled */}
          {wd.pending.length > 0 && (
            <div className="mt-4 rounded-[24px] p-5 backdrop-blur-xl border border-white/10 bg-white/[0.03]">
              <p className="text-[11px] text-white/40 uppercase tracking-[0.18em] mb-3">
                Scheduled cashouts
              </p>
              <div className="space-y-2">
                {wd.pending.map((req) => (
                  <PendingRow
                    key={req.requestId}
                    req={req}
                    claimable={wd.isClaimable(req)}
                    remainingMs={wd.cooldownRemaining(req)}
                    isClaiming={wd.isClaiming && wd.claimingId === req.requestId}
                    onClaim={() => wd.claimRedeem(req.requestId)}
                  />
                ))}
              </div>
            </div>
          )}

          {/* Recent activity — localStorage-backed tx history */}
          {/* "Since you started" summary — lifetime earnings estimated
              from local tx history vs current balance. Anchored right
              above the activity log so the two read as one narrative:
              "here's what you've earned total, and here's the list of
              every step that got you there." */}
          {txHistory.hasHistory && hasRealBalance && (
            <SinceYouStartedCard
              history={txHistory.history}
              currentUsdc={usdcValue}
            />
          )}

          {txHistory.hasHistory && (
            <RecentActivity history={txHistory.history} />
          )}

          {/* Glassbox Panel — "See how your Dol works" */}
          {phase === "active" && hasRealBalance && (
            <GlassboxPanel />
          )}

          {/* Legal — same fix as the landing page disclaimer. Previous
              color: #333333 on black was effectively invisible (~1.5:1
              contrast, fails WCAG AA). Risk warnings that users can't
              read are a legal/liability issue, not a stylistic choice. */}
          <div className="mt-8 px-2 text-center text-[11px] leading-relaxed text-white/40">
            Dol is not a bank. Each Dol is backed 1:1 by USDC. Numbers shown
            are a target, not a promise. Capital is at risk. Not available
            in the United States, Korea, or other restricted jurisdictions.
          </div>
        </div>
      </motion.section>

      <CashoutSheet
        open={cashoutOpen}
        balance={usdcValue}
        instantBufferShort={instantBufferShort}
        onClose={() => setCashoutOpen(false)}
        onScheduled={(amount) => {
          const shares = toShares(amount, balanceShares);
          if (shares <= BigInt(0)) {
            toast.error("Invalid amount.", {
              description: "Enter a number between 0 and your balance.",
            });
            return;
          }
          wd.requestRedeem(shares);
        }}
        onInstant={(amount) => {
          const shares = toShares(amount, balanceShares);
          if (shares <= BigInt(0)) {
            toast.error("Invalid amount.", {
              description: "Enter a number between 0 and your balance.",
            });
            return;
          }
          wd.instantRedeem(shares);
        }}
      />

      <style jsx global>{`
        @keyframes shimmerGradient {
          0%,
          100% {
            background-position: 0% 50%;
          }
          50% {
            background-position: 100% 50%;
          }
        }
      `}</style>
      <SiteFooter />
    </main>
  );
}

/* SinceYouStartedCard — lifetime earnings estimate built from the
   localStorage tx history and the current on-chain balance.

   Math (best-effort, no backend):

     totalIn     = sum of confirmed deposit.amount
     totalOutSched = sum of redeem-scheduled.amount that later matched
                     a claim (we approximate by summing all claims)
     totalOutInstant = sum of redeem-instant.amount
     lifetimeOut = totalOutInstant + totalOutClaimed
     earned ≈ currentUsdc + lifetimeOut - totalIn

   This is directionally correct even if tx history is incomplete
   (e.g., the user cleared localStorage mid-session) — we clamp the
   result to ≥ 0 and label it "so far" so the number never reads as
   an authoritative ledger. If tx history is thin (fewer than 1
   deposit), we hide the card entirely to avoid surfacing a
   meaningless "$0.00 earned so far".

   First-deposit timestamp drives the "Since you started" age label.
*/
function SinceYouStartedCard({
  history,
  currentUsdc,
}: {
  history: LoggedTx[];
  currentUsdc: number;
}) {
  const deposits = history.filter((h) => h.type === "deposit");
  if (deposits.length === 0) return null;

  const totalIn = deposits.reduce((sum, h) => sum + (h.amount || 0), 0);
  const totalOutInstant = history
    .filter((h) => h.type === "redeem-instant")
    .reduce((sum, h) => sum + (h.amount || 0), 0);
  const totalOutClaimed = history
    .filter((h) => h.type === "claim")
    .reduce((sum, h) => sum + (h.amount || 0), 0);
  const lifetimeOut = totalOutInstant + totalOutClaimed;
  const earned = Math.max(currentUsdc + lifetimeOut - totalIn, 0);

  // Earliest deposit timestamp — drives the age label.
  const firstDepositTs = Math.min(...deposits.map((h) => h.timestamp));
  const ageLabel = formatLongAge(Date.now() - firstDepositTs);

  return (
    <div className="mt-4 rounded-[24px] border border-white/10 bg-white/[0.03] p-5 backdrop-blur-xl">
      <div className="flex items-baseline justify-between">
        <p className="text-[11px] uppercase tracking-[0.18em] text-white/40">
          Since you started
        </p>
        <p className="text-[11px] text-white/35">{ageLabel}</p>
      </div>
      <div className="mt-3 flex items-baseline gap-3 whitespace-nowrap">
        <span
          className="font-bold leading-none text-white tabular-nums"
          style={{
            // Same defensive clamp as the balance: at a small phone
            // the earnings figure might exceed 8 characters ("+$1.2345
            // earned" is 8, "+$123.4567" is 10) and a hard 34 px would
            // push past the 239 px content width of the card.
            fontSize: "clamp(22px, 7.5vw, 34px)",
            letterSpacing: "-0.02em",
          }}
        >
          +$
          {earned.toLocaleString("en-US", {
            minimumFractionDigits: 4,
            maximumFractionDigits: 4,
          })}
        </span>
        <span className="text-[12px] leading-none text-white/40">earned</span>
      </div>
      <p className="mt-3 text-[11px] leading-relaxed text-white/45">
        You&apos;ve put in $
        {totalIn.toLocaleString("en-US", {
          minimumFractionDigits: 2,
          maximumFractionDigits: 2,
        })}
        {lifetimeOut > 0 && (
          <>
            {" "}
            and cashed out $
            {lifetimeOut.toLocaleString("en-US", {
              minimumFractionDigits: 2,
              maximumFractionDigits: 2,
            })}
          </>
        )}
        .{" "}
        {earned > 0.0001
          ? "The rest is your Dol growing on its own."
          : "Growth takes a little time — come back in a day."}
      </p>
    </div>
  );
}

// Long-form "X days ago" label for the Since You Started header.
// Deliberately separate from formatRelativeTime because the framing
// is "how long you've been in" rather than "how long ago did this
// happen" — phrased as duration, not relative point in time.
function formatLongAge(ms: number): string {
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return "moments ago";
  if (sec < 3600) {
    const m = Math.floor(sec / 60);
    return `${m} minute${m === 1 ? "" : "s"}`;
  }
  if (sec < 86400) {
    const h = Math.floor(sec / 3600);
    return `${h} hour${h === 1 ? "" : "s"}`;
  }
  const d = Math.floor(sec / 86400);
  return `${d} day${d === 1 ? "" : "s"}`;
}

/* RecentActivity — localStorage-backed tx log. Collapsible card below
   Pending Requests. Shows most recent 10; rest hidden behind "Show all". */
function RecentActivity({ history }: { history: LoggedTx[] }) {
  const [expanded, setExpanded] = useState(false);
  const visible = expanded ? history : history.slice(0, 5);

  return (
    <div className="mt-4 rounded-[24px] p-5 backdrop-blur-xl border border-white/10 bg-white/[0.03]">
      <p className="text-[11px] text-white/40 uppercase tracking-[0.18em] mb-3">
        Recent activity
      </p>
      <div className="space-y-2">
        {visible.map((tx) => (
          <ActivityRow key={tx.hash} tx={tx} />
        ))}
      </div>
      {history.length > 5 && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="mt-3 w-full py-2 rounded-full text-[11px] text-white/40 hover:text-white/70 transition-colors"
        >
          {expanded
            ? "Show recent only"
            : `Show all (${history.length})`}
        </button>
      )}
    </div>
  );
}

function ActivityRow({ tx }: { tx: LoggedTx }) {
  // Plain-English copy: every sentence starts with "You …" so a
  // first-time user instantly understands what each row represents
  // without having to translate "redeem-scheduled" or "approve" in
  // their head. Amount lives inside the headline, not in the meta
  // row, because $100 is the thing they care about most.
  const amountStr =
    tx.amount > 0 ? `${tx.amount.toFixed(2)} Dol` : "";
  const sentence = (() => {
    switch (tx.type) {
      case "deposit":
        return amountStr ? `You bought ${amountStr}` : "You bought Dol";
      case "redeem-scheduled":
        return amountStr
          ? `You scheduled a cash out of ${amountStr}`
          : "You scheduled a cash out";
      case "redeem-instant":
        return amountStr
          ? `You cashed out ${amountStr} instantly`
          : "You cashed out instantly";
      case "claim":
        return amountStr
          ? `You received ${amountStr} back`
          : "You received your cash out";
      case "approve":
        return "You allowed Dol to use your USDC";
      default:
        return "Activity";
    }
  })();

  const age = formatRelativeTime(tx.timestamp);
  const statusLabel =
    tx.status === "confirmed"
      ? "Confirmed"
      : tx.status === "failed"
        ? "Failed"
        : "Pending";
  const statusColor =
    tx.status === "confirmed"
      ? "text-emerald-400/80"
      : tx.status === "failed"
        ? "text-red-400/80"
        : "text-white/45";

  return (
    <a
      href={`https://sepolia.basescan.org/tx/${tx.hash}`}
      target="_blank"
      rel="noopener noreferrer"
      className="flex items-center justify-between rounded-xl bg-white/[0.02] border border-white/5 px-4 py-2.5 hover:border-white/15 transition-colors group"
    >
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-white/85 truncate">
          {sentence}
        </div>
        <div className="mt-0.5 text-[11px] text-white/40">
          {age} &middot;{" "}
          <span className={statusColor}>{statusLabel}</span>
        </div>
      </div>
      <span className="text-[10px] text-white/20 group-hover:text-white/50 transition-colors ml-3 flex-shrink-0">
        View &rarr;
      </span>
    </a>
  );
}

// Natural-language relative time — avoids abbreviated "5m ago" in
// favor of the full word, which feels more like a person talking to
// the user than a system log line.
function formatRelativeTime(ts: number): string {
  const sec = Math.floor((Date.now() - ts) / 1000);
  if (sec < 30) return "just now";
  if (sec < 60) return `${sec} seconds ago`;
  if (sec < 120) return "a minute ago";
  if (sec < 3600) return `${Math.floor(sec / 60)} minutes ago`;
  if (sec < 7200) return "an hour ago";
  if (sec < 86400) return `${Math.floor(sec / 3600)} hours ago`;
  if (sec < 172800) return "yesterday";
  if (sec < 604800) return `${Math.floor(sec / 86400)} days ago`;
  return `${Math.floor(sec / 604800)} weeks ago`;
}

/* PendingRow — a single scheduled redeem with countdown + claim button */
function PendingRow({
  req,
  claimable,
  remainingMs,
  isClaiming,
  onClaim,
}: {
  req: PendingRedeem;
  claimable: boolean;
  remainingMs: number;
  isClaiming: boolean;
  onClaim: () => void;
}) {
  // Re-render every second while waiting for countdown
  const [, tick] = useState(0);
  useEffect(() => {
    if (claimable) return;
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, [claimable]);

  // Clamp to 0 — otherwise during the 1-second window between the
  // cooldown actually elapsing and `claimable` flipping true in the
  // parent, this renders "-0m -1s" which reads as a broken UI.
  const safeRemaining = Math.max(0, remainingMs);
  const mins = Math.floor(safeRemaining / 60000);
  const secs = Math.floor((safeRemaining % 60000) / 1000);
  const countdownStr =
    mins > 60
      ? `${Math.floor(mins / 60)}h ${mins % 60}m`
      : `${mins}m ${secs}s`;

  return (
    <div className="flex items-center justify-between rounded-xl bg-white/[0.03] border border-white/5 px-4 py-3">
      <div>
        <div className="text-[14px] font-medium text-white tabular-nums">
          {req.shares.toFixed(4)} Dol
        </div>
        <div className="mt-0.5 flex items-center gap-1 text-[11px] text-white/40">
          {claimable ? (
            <>
              <CheckCircle className="h-3 w-3" />
              Ready
            </>
          ) : (
            <>
              <Clock className="h-3 w-3" />
              Ready in {countdownStr}
            </>
          )}
        </div>
      </div>
      <button
        onClick={onClaim}
        disabled={!claimable || isClaiming}
        className={`rounded-full px-4 py-1.5 text-[12px] font-semibold transition-colors ${
          claimable
            ? "bg-white text-black hover:bg-white/90"
            : "bg-white/10 text-white/40 cursor-not-allowed"
        }`}
      >
        {isClaiming ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : (
          "Claim"
        )}
      </button>
    </div>
  );
}

/* GlassboxPanel — "See how your Dol works"
   Tap-to-expand transparency panel showing where funds are working.
   No individual transactions (privacy). Just a visual light-flow
   of money splitting into 2 safe engines and coming back.
   Tiny [View on Explorer] link in the bottom right for the curious. */
function GlassboxPanel() {
  const [open, setOpen] = useState(false);

  return (
    <div className="mt-5">
      <motion.button
        whileTap={{ scale: 0.97 }}
        transition={SOFT_SPRING}
        onClick={() => setOpen(!open)}
        className="w-full py-3 rounded-full text-[12px] text-white/50 hover:text-white/80 transition-colors flex items-center justify-center gap-2"
      >
        <span>{open ? "Hide" : "See how your Dol works"}</span>
        <motion.span
          animate={{ rotate: open ? 180 : 0 }}
          transition={SOFT_SPRING}
          className="inline-block text-[10px]"
        >
          &darr;
        </motion.span>
      </motion.button>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.5, ease: APPLE_EASE }}
            className="overflow-hidden"
          >
            <div
              className="mt-3 rounded-[28px] p-6 backdrop-blur-2xl border border-white/10"
              style={{
                background:
                  "linear-gradient(180deg, rgba(255,255,255,0.03) 0%, rgba(255,255,255,0.01) 100%)",
              }}
            >
              <p className="text-[11px] text-white/35 uppercase tracking-[0.18em] text-center">
                Your Dol is working in
              </p>

              {/* Light-flow SVG */}
              <div className="mt-5">
                <LightFlowSVG />
              </div>

              <div className="mt-6 space-y-3">
                <EngineRow
                  pct="70%"
                  name="Safe market engines"
                  detail="Protected against price swings"
                />
                <EngineRow
                  pct="30%"
                  name="Over-collateralized lending"
                  detail="Backed by more than 100% collateral"
                />
              </div>

              <p className="mt-6 text-[12px] text-white/40 text-center leading-relaxed">
                Zero exposure to price direction.
                <br />
                Always working. Always safe.
              </p>

              {/* Tiny explorer link — for the curious */}
              <div className="mt-5 flex justify-end">
                <a
                  href="https://sepolia.basescan.org"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-[10px] text-white/25 hover:text-white/50 transition-colors flex items-center gap-1"
                >
                  View on Explorer
                  <span className="text-[9px]">&rarr;</span>
                </a>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function EngineRow({
  pct,
  name,
  detail,
}: {
  pct: string;
  name: string;
  detail: string;
}) {
  return (
    <div className="flex items-center justify-between px-3 py-2.5 rounded-xl bg-white/[0.03] border border-white/5">
      <div className="flex items-center gap-3">
        <span className="text-[14px] font-semibold text-white/90 tabular-nums min-w-[42px]">
          {pct}
        </span>
        <div>
          <div className="text-[13px] font-medium text-white/80">{name}</div>
          <div className="text-[11px] text-white/35">{detail}</div>
        </div>
      </div>
      <span className="h-1.5 w-1.5 rounded-full bg-white/40 animate-pulse" />
    </div>
  );
}

/* LightFlowSVG — money flows from center YOU, splits into 2 engines,
   earnings dots pulse back to YOU. Looping SVG animation via
   stroke-dashoffset + SMIL <animate>. No Three.js. */
function LightFlowSVG() {
  return (
    <svg
      viewBox="0 0 360 140"
      className="w-full h-auto"
      role="img"
      aria-label="Flow of funds to safe engines"
    >
      <defs>
        <linearGradient id="flowLine" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0" />
          <stop offset="50%" stopColor="#ffffff" stopOpacity="0.7" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </linearGradient>
        <radialGradient id="flowNode" cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0.35" />
          <stop offset="60%" stopColor="#ffffff" stopOpacity="0.06" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </radialGradient>
      </defs>

      {/* Base curves — faint */}
      <path
        d="M 180 70 Q 265 30, 340 40"
        stroke="#ffffff"
        strokeOpacity="0.08"
        strokeWidth="1"
        fill="none"
      />
      <path
        d="M 180 70 Q 265 110, 340 100"
        stroke="#ffffff"
        strokeOpacity="0.08"
        strokeWidth="1"
        fill="none"
      />

      {/* Outgoing flow trails */}
      <path
        d="M 180 70 Q 265 30, 340 40"
        stroke="url(#flowLine)"
        strokeWidth="1.8"
        fill="none"
        strokeLinecap="round"
        strokeDasharray="24 180"
      >
        <animate
          attributeName="stroke-dashoffset"
          from="204"
          to="0"
          dur="3s"
          repeatCount="indefinite"
        />
      </path>
      <path
        d="M 180 70 Q 265 110, 340 100"
        stroke="url(#flowLine)"
        strokeWidth="1.8"
        fill="none"
        strokeLinecap="round"
        strokeDasharray="24 180"
      >
        <animate
          attributeName="stroke-dashoffset"
          from="204"
          to="0"
          dur="3.4s"
          repeatCount="indefinite"
        />
      </path>

      {/* Earning dots pulsing back */}
      <circle cx="340" cy="40" r="3" fill="#ffffff" fillOpacity="0.8">
        <animate
          attributeName="cx"
          values="340;180"
          dur="3s"
          begin="1.2s"
          repeatCount="indefinite"
        />
        <animate
          attributeName="cy"
          values="40;70"
          dur="3s"
          begin="1.2s"
          repeatCount="indefinite"
        />
        <animate
          attributeName="opacity"
          values="0;1;1;0"
          dur="3s"
          begin="1.2s"
          repeatCount="indefinite"
        />
      </circle>
      <circle cx="340" cy="100" r="3" fill="#ffffff" fillOpacity="0.8">
        <animate
          attributeName="cx"
          values="340;180"
          dur="3.4s"
          begin="1.5s"
          repeatCount="indefinite"
        />
        <animate
          attributeName="cy"
          values="100;70"
          dur="3.4s"
          begin="1.5s"
          repeatCount="indefinite"
        />
        <animate
          attributeName="opacity"
          values="0;1;1;0"
          dur="3.4s"
          begin="1.5s"
          repeatCount="indefinite"
        />
      </circle>

      {/* Center node — YOU */}
      <circle cx="180" cy="70" r="28" fill="url(#flowNode)" />
      <circle
        cx="180"
        cy="70"
        r="14"
        fill="none"
        stroke="#ffffff"
        strokeOpacity="0.5"
        strokeWidth="1"
      />
      <circle cx="180" cy="70" r="2" fill="#ffffff" fillOpacity="0.9" />
      <text
        x="180"
        y="112"
        textAnchor="middle"
        fill="#ffffff"
        fillOpacity="0.6"
        fontSize="9"
        fontWeight="500"
        letterSpacing="0.1em"
      >
        YOU
      </text>

      {/* Right top — Engine 1 */}
      <circle cx="340" cy="40" r="22" fill="url(#flowNode)" />
      <circle
        cx="340"
        cy="40"
        r="10"
        fill="none"
        stroke="#ffffff"
        strokeOpacity="0.4"
        strokeWidth="1"
      />
      <text
        x="340"
        y="18"
        textAnchor="middle"
        fill="#ffffff"
        fillOpacity="0.5"
        fontSize="8"
        letterSpacing="0.08em"
      >
        70%
      </text>

      {/* Right bottom — Engine 2 */}
      <circle cx="340" cy="100" r="22" fill="url(#flowNode)" />
      <circle
        cx="340"
        cy="100"
        r="10"
        fill="none"
        stroke="#ffffff"
        strokeOpacity="0.4"
        strokeWidth="1"
      />
      <text
        x="340"
        y="132"
        textAnchor="middle"
        fill="#ffffff"
        fillOpacity="0.5"
        fontSize="8"
        letterSpacing="0.08em"
      >
        30%
      </text>
    </svg>
  );
}

export default function MyDolPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-screen bg-black flex items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-white/30" />
        </div>
      }
    >
      <MyDolInner />
    </Suspense>
  );
}
