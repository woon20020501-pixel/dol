"use client";

import { useState, useEffect, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import {
  useAccount,
  useReadContract,
  useWriteContract,
  useWaitForTransactionReceipt,
} from "wagmi";
import { parseUnits, maxUint256 } from "viem";
import { baseSepolia } from "wagmi/chains";
import { usePrivy, useWallets } from "@privy-io/react-auth";
import {
  CheckCircle,
  ExternalLink,
  Loader2,
  ArrowLeft,
  AlertTriangle,
} from "lucide-react";
import { toast } from "sonner";
import { getPBondConfig } from "@/lib/pbond";
import { ERC20_ABI } from "@/lib/vault";
import { emitDolTxConfirmed } from "@/lib/txEvents";
import {
  translateError,
  BASE_SEPOLIA_USDC_FAUCET,
  BASE_SEPOLIA_ETH_FAUCET,
  type ErrorCategory,
} from "@/lib/errors";
import DolHeroImage from "@/components/DolHeroImage";
import LiveCounter from "@/components/LiveCounter";
import WalletChip from "@/components/WalletChip";
import { SiteFooter } from "@/components/SiteFooter";
import { useTosAcceptance } from "@/components/FirstDepositClickwrap";
import { Glossary } from "@/components/Glossary";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useTxHistory } from "@/hooks/useTxHistory";

const BASESCAN = "https://sepolia.basescan.org";
const USDC_DECIMALS = 6;
const TARGET_CHAIN_ID = baseSepolia.id;
const DOL_APY = 0.075;

function DepositPageInner() {
  useKeyboardShortcuts();
  useSearchParams(); // keep for Suspense boundary
  const { authenticated, login, ready } = usePrivy();
  const { address: userAddress } = useAccount();
  const { wallets } = useWallets();
  const [walletChainId, setWalletChainId] = useState<number | null>(null);
  const [amount, setAmount] = useState("");
  const txHistory = useTxHistory();
  // Layer B clickwrap — gates the first deposit per wallet on
  // TOS/Privacy/Risk acceptance. Modal is rendered further down.
  const { requireTos, modal: tosModal } = useTosAcceptance(userAddress);

  const config = getPBondConfig();
  const target = config.senior; // Dol only uses senior tranche; junior hidden

  // Upper cap (10 M USDC) — same semantic as a "sensible max" check.
  // Any input larger than this is either a typo or an attempt to
  // trigger a huge approve for downstream exploitation. We clamp
  // here so the downstream `parseUnits` call never sees a number
  // outside Number.MAX_SAFE_INTEGER / 1e6.
  const MAX_AMOUNT_USDC = 10_000_000;
  const numAmount = (() => {
    const raw = Number(amount);
    if (!Number.isFinite(raw) || raw < 0) return 0;
    return Math.min(raw, MAX_AMOUNT_USDC);
  })();

  // `parseUnits` throws on malformed input — scientific notation
  // ("1e5"), negative values, fractional components beyond the
  // token's decimals ("0.0000001" at 6 decimals), etc. Wrap in
  // try/catch so a hostile paste never crashes the route and kicks
  // the user into the global error boundary.
  const parsedAmount = (() => {
    if (numAmount <= 0) return BigInt(0);
    try {
      return parseUnits(numAmount.toString(), USDC_DECIMALS);
    } catch {
      return BigInt(0);
    }
  })();

  // Track Privy wallet chain
  const activeWallet = wallets[0];
  useEffect(() => {
    if (activeWallet) {
      const cid = Number(
        activeWallet.chainId.split(":")[1] ?? activeWallet.chainId
      );
      setWalletChainId(cid);
    }
  }, [activeWallet]);

  const isWrongChain =
    authenticated && walletChainId !== null && walletChainId !== TARGET_CHAIN_ID;

  const handleSwitchChain = async () => {
    if (!activeWallet) return;
    try {
      await activeWallet.switchChain(TARGET_CHAIN_ID);
      setWalletChainId(TARGET_CHAIN_ID);
    } catch (e) {
      console.error("[Dol] Chain switch failed:", e);
    }
  };

  useEffect(() => {
    if (isWrongChain && activeWallet) handleSwitchChain();
  }, [isWrongChain, activeWallet]); // eslint-disable-line react-hooks/exhaustive-deps

  // USDC balance
  const { data: usdcBalanceRaw } = useReadContract({
    address: config.usdcAddress,
    abi: ERC20_ABI,
    functionName: "balanceOf",
    args: userAddress ? [userAddress] : undefined,
    chainId: TARGET_CHAIN_ID,
    query: { enabled: !!userAddress },
  });
  const usdcBalance =
    usdcBalanceRaw !== undefined ? Number(usdcBalanceRaw) / 1e6 : null;

  // Allowance
  const { data: allowanceRaw, refetch: refetchAllowance } = useReadContract({
    address: config.usdcAddress,
    abi: ERC20_ABI,
    functionName: "allowance",
    args: userAddress ? [userAddress, target.address] : undefined,
    chainId: TARGET_CHAIN_ID,
    query: { enabled: !!userAddress },
  });
  const needsApproval =
    !allowanceRaw || (parsedAmount > BigInt(0) && allowanceRaw < parsedAmount);

  // ── Phase 1 TVL cap enforcement ────────────────────────────────
  //
  // The framework-assumptions doc published a $100,000 hard TVL
  // ceiling for Phase 1, derived from Pacifica's Closed Beta
  // per-account equity cap. The doc promised "deposits will be
  // limited and the frontend will surface a clear capacity-reached
  // state instead of silently accepting money." Until the contracts
  // layer adds a contract-level maxTotalAssets, this is that enforcement at
  // the frontend layer: we read the Dol contract's current totalSupply
  // (which tracks USDC value at near-1:1 during Phase 1), compute
  // how much headroom is left under the cap, and gate the approve
  // and deposit buttons against it.
  //
  // This is defense in depth, not a security boundary — a user who
  // calls the contract directly with cast/ethers will bypass the
  // frontend check. Phase 1 risk budget accepts that because the
  // real cap is Pacifica's own per-account limit on the operator
  // side. When mainnet lands, the cap should move into the contract.
  //
  const PHASE_1_TVL_CAP_USDC = 100_000;
  const { data: totalSupplyRaw } = useReadContract({
    address: target.address,
    abi: target.abi,
    functionName: "totalSupply",
    chainId: TARGET_CHAIN_ID,
    // Poll every 15 s so a fresh deposit elsewhere in the wild is
    // reflected in our cap banner within the same hold interval.
    // The 15 s cadence also catches our own deposit within one read
    // cycle after confirmation, so we don't need a manual refetch
    // in the confirmation effect.
    query: { enabled: !!target.address, refetchInterval: 15_000 },
  });
  // Dol is 6-decimal and ~1:1 backed. Using totalSupply directly as a
  // USDC-equivalent TVL proxy is accurate to < 0.1 % at Phase 1 growth
  // rates. Precise conversion via pricePerShare would be more correct
  // but the error is well below the cap buffer we want anyway.
  const currentTvlUsdc =
    typeof totalSupplyRaw === "bigint"
      ? Number(totalSupplyRaw) / 1e6
      : null;
  const tvlRemaining =
    currentTvlUsdc !== null
      ? Math.max(0, PHASE_1_TVL_CAP_USDC - currentTvlUsdc)
      : null;
  const tvlCapReached = tvlRemaining !== null && tvlRemaining <= 0;
  const amountExceedsCap =
    tvlRemaining !== null && numAmount > tvlRemaining;

  // Approve
  const {
    writeContract: approve,
    data: approveTxHash,
    isPending: isApproving,
    reset: resetApprove,
    error: approveError,
  } = useWriteContract();
  const { isSuccess: isApproveConfirmed } = useWaitForTransactionReceipt({
    hash: approveTxHash,
    chainId: TARGET_CHAIN_ID,
  });
  useEffect(() => {
    if (isApproveConfirmed) {
      refetchAllowance();
      emitDolTxConfirmed("approve", approveTxHash);
    }
  }, [isApproveConfirmed, refetchAllowance, approveTxHash]);

  // Deposit
  const {
    writeContract: deposit,
    data: depositTxHash,
    isPending: isDepositing,
    reset: resetDeposit,
    error: depositError,
  } = useWriteContract();
  const { isSuccess: isDepositConfirmed } = useWaitForTransactionReceipt({
    hash: depositTxHash,
    chainId: TARGET_CHAIN_ID,
  });

  // Record successful deposit in local history + broadcast tx confirm
  useEffect(() => {
    if (isDepositConfirmed && depositTxHash) {
      try {
        txHistory.record({
          hash: depositTxHash,
          type: "deposit",
          amount: Number.isFinite(numAmount) ? numAmount : 0,
        });
      } catch {
        // Local history is best-effort; swallow failures silently.
      }
      emitDolTxConfirmed("deposit", depositTxHash);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isDepositConfirmed, depositTxHash]);

  // Runtime pre-flight check run before every write. Refuses to
  // fire if any of the following are true:
  //
  //   - parsedAmount ≤ 0 (defensive — UI disables the button but
  //     a scripted click could still reach here)
  //   - parsedAmount exceeds the user's on-chain balance
  //   - the wallet is on the wrong chain (we auto-switch elsewhere,
  //     but we re-verify here just before signing)
  //   - the target contract address is malformed
  //
  // Returns a reason string on failure, null on success. Each
  // handler below calls it and aborts with a toast on non-null.
  const preflightReject = (): string | null => {
    if (parsedAmount <= BigInt(0)) return "Enter an amount first.";
    if (usdcBalance !== null && numAmount > usdcBalance) {
      return "Not enough USDC in your wallet.";
    }
    if (walletChainId !== null && walletChainId !== TARGET_CHAIN_ID) {
      return "Wrong network. Switch to Base Sepolia.";
    }
    if (!/^0x[0-9a-fA-F]{40}$/.test(target.address)) {
      return "Contract address is invalid. Please refresh.";
    }
    if (!/^0x[0-9a-fA-F]{40}$/.test(config.usdcAddress)) {
      return "USDC address is invalid. Please refresh.";
    }
    if (tvlCapReached) {
      return `Capacity reached. Dol is at its Phase 1 TVL cap of $${PHASE_1_TVL_CAP_USDC.toLocaleString(
        "en-US",
      )}.`;
    }
    if (amountExceedsCap && tvlRemaining !== null) {
      return `Too much. Only $${tvlRemaining.toLocaleString("en-US", {
        maximumFractionDigits: 2,
      })} of capacity remaining under the Phase 1 cap.`;
    }
    return null;
  };

  const handleApprove = () => {
    const reason = preflightReject();
    if (reason) {
      toast.error(reason);
      return;
    }
    requireTos(() => {
      resetApprove();
      // Approve max uint256 — user approves once and any future deposit
      // (same session or later) skips the 2-step flow. Also side-steps
      // the post-approval allowance-refetch race that was causing the
      // "two clicks needed, half the time deposits fail" UX bug on the
      // standalone deposit page.
      approve({
        address: config.usdcAddress,
        abi: ERC20_ABI,
        functionName: "approve",
        args: [target.address, maxUint256],
        chainId: TARGET_CHAIN_ID,
      });
    });
  };

  const handleDeposit = () => {
    const reason = preflightReject();
    if (reason) {
      toast.error(reason);
      return;
    }
    requireTos(() => {
      resetDeposit();
      deposit({
        address: target.address,
        abi: target.abi,
        functionName: "deposit",
        args: [parsedAmount],
        chainId: TARGET_CHAIN_ID,
      });
    });
  };

  const insufficientBalance =
    usdcBalance !== null && numAmount > usdcBalance;
  const inputInvalid = !amount || numAmount <= 0;

  // Error toast — plain-English copy, faucet link for specific categories.
  // `lastErrorCategory` feeds into the optional helper link row below the form.
  const [lastErrorCategory, setLastErrorCategory] =
    useState<ErrorCategory | null>(null);

  useEffect(() => {
    const rawErr = approveError || depositError;
    if (!rawErr) return;
    const e = translateError(rawErr);
    setLastErrorCategory(e.category);

    // User-rejected is a non-event — show a light neutral toast, no "error".
    if (e.category === "user_rejected") {
      toast(e.title);
      return;
    }

    toast.error(e.title, {
      description: e.description,
      action:
        e.category === "insufficient_usdc"
          ? {
              label: "Get test USDC",
              onClick: () =>
                window.open(BASE_SEPOLIA_USDC_FAUCET, "_blank", "noopener,noreferrer"),
            }
          : e.category === "insufficient_eth"
            ? {
                label: "Get test ETH",
                onClick: () =>
                  window.open(BASE_SEPOLIA_ETH_FAUCET, "_blank", "noopener,noreferrer"),
              }
            : undefined,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [approveError, depositError]);

  // Clear the helper hint once the user touches the input again
  useEffect(() => {
    if (lastErrorCategory) setLastErrorCategory(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [amount]);

  return (
    <main className="min-h-screen bg-black text-white">
      {/* Header */}
      <header className="fixed top-0 left-0 right-0 z-50 bg-black/80 backdrop-blur-xl border-b border-white/5">
        <div className="mx-auto flex h-[56px] max-w-[1080px] items-center justify-between px-6">
          <Link href="/" className="text-[20px] font-semibold tracking-tight">
            Dol
          </Link>
          <nav className="flex items-center gap-4" aria-label="Primary">
            <Link
              href="/"
              className="text-[13px] text-white/50 hover:text-white transition-colors"
            >
              <ArrowLeft className="inline h-3.5 w-3.5 mr-1" />
              Back
            </Link>
            <Link
              href="/docs"
              className="text-[13px] text-white/70 hover:text-white transition-colors"
            >
              Docs
            </Link>
            <Link
              href="/faq"
              className="text-[13px] text-white/70 hover:text-white transition-colors"
            >
              FAQ
            </Link>
            {!authenticated && ready && (
              <button
                onClick={login}
                className="rounded-full bg-white px-5 py-2 text-[13px] font-medium text-black hover:bg-white/90 transition-colors"
              >
                Connect
              </button>
            )}
            <WalletChip />
          </nav>
        </div>
      </header>

      <div className="mx-auto max-w-[540px] px-6 pt-32 pb-20">
        {isDepositConfirmed && depositTxHash ? (
          /* Success state */
          <div className="text-center">
            <CheckCircle className="mx-auto h-14 w-14 text-white mb-6" />
            <h1
              className="text-5xl md:text-6xl font-bold text-white leading-[1]"
              style={{ letterSpacing: "-0.04em" }}
            >
              You have a Dol.
            </h1>
            <p className="mt-4 text-lg text-white/50">
              Your {amount} Dol is growing right now.
            </p>
            <div className="mt-10 text-4xl font-semibold tabular-nums">
              <LiveCounter
                initial={Number(amount)}
                apy={DOL_APY}
                decimals={6}
              />
              <span className="text-white/40 text-2xl ml-2 font-semibold">
                Dol
              </span>
            </div>
            <a
              href={`${BASESCAN}/tx/${depositTxHash}`}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-8 inline-flex items-center gap-1.5 text-sm text-white/50 hover:text-white transition-colors"
            >
              View on Basescan
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
            <div className="mt-10 flex flex-col gap-3 sm:flex-row sm:justify-center">
              <Link
                href="/my-dol?fresh=1"
                className="rounded-full bg-white px-8 py-3.5 text-[15px] font-semibold text-black hover:bg-white/90 transition-colors text-center"
                style={{
                  letterSpacing: "-0.01em",
                  boxShadow: "0 14px 40px rgba(255,255,255,0.15)",
                }}
              >
                View your Dol &rarr;
              </Link>
              <button
                onClick={() => {
                  setAmount("");
                  resetDeposit();
                  resetApprove();
                }}
                className="rounded-full border border-white/20 px-6 py-3 text-[15px] font-medium text-white hover:bg-white/5 transition-colors"
              >
                Buy more
              </button>
            </div>
          </div>
        ) : (
          <>
            {/* Hero image */}
            <div className="flex justify-center mb-8">
              <DolHeroImage size={220} />
            </div>

            <h1
              className="text-5xl md:text-6xl font-bold text-white text-center leading-[1]"
              style={{ letterSpacing: "-0.04em" }}
            >
              Get a Dol.
            </h1>
            <p className="mt-4 text-lg text-white/50 text-center">
              Grows up to 7.5% a year. Cash out anytime.
            </p>
            <p className="mt-2 text-[12px] text-white/30 text-center tracking-[0.02em]">
              1 Dol = 1 <Glossary term="usdc">USDC</Glossary>. Always.
            </p>

            {/* Chain switch banner */}
            {isWrongChain && (
              <div className="mt-8 rounded-2xl border border-amber-400/20 bg-amber-400/5 px-5 py-4 text-center">
                <AlertTriangle className="mx-auto h-5 w-5 text-amber-400 mb-2" />
                <p className="text-sm text-amber-200">
                  Wrong network (chain {walletChainId}). Switch to Base Sepolia.
                </p>
                <button
                  onClick={handleSwitchChain}
                  className="mt-3 rounded-full bg-amber-400 px-5 py-2 text-sm font-medium text-black hover:bg-amber-300 transition-colors"
                >
                  Switch network
                </button>
              </div>
            )}

            {/* Amount input */}
            <div className="mt-10 rounded-2xl bg-white/[0.04] border border-white/10 p-6">
              <div className="flex items-center justify-between mb-2">
                <label
                  htmlFor="amount"
                  className="text-xs uppercase tracking-widest text-white/40"
                >
                  How much
                </label>
                {usdcBalance !== null && (
                  <button
                    type="button"
                    onClick={() => setAmount(usdcBalance.toString())}
                    className="text-xs text-white/50 hover:text-white transition-colors"
                  >
                    Balance: {usdcBalance.toFixed(2)} USDC
                  </button>
                )}
              </div>
              <div className="relative">
                {/* text + inputMode decimal + manual sanitization —
                    `type="number"` accepts scientific notation like
                    "1e18" and leaves the raw string to the parser
                    downstream, which then throws inside parseUnits
                    and crashes the route. We strip everything that
                    isn't a digit or a single dot, cap at 6 decimals
                    (USDC precision), and accept max 9 integer digits
                    so `Number()` never exceeds MAX_SAFE_INTEGER. */}
                <input
                  id="amount"
                  type="text"
                  inputMode="decimal"
                  autoComplete="off"
                  placeholder="0"
                  value={amount}
                  onChange={(e) => {
                    const clean = (() => {
                      // Keep only digits and at most one dot.
                      const digits = e.target.value.replace(/[^\d.]/g, "");
                      const parts = digits.split(".");
                      const whole = parts[0].slice(0, 9); // 9 integer digits cap
                      if (parts.length < 2) return whole;
                      // Single decimal + up to 6 fractional digits.
                      const frac = parts.slice(1).join("").slice(0, 6);
                      return `${whole}.${frac}`;
                    })();
                    setAmount(clean);
                  }}
                  className="w-full bg-transparent text-4xl font-semibold text-white placeholder:text-white/20 focus:outline-none tabular-nums pr-20"
                />
                <span className="absolute right-0 top-1/2 -translate-y-1/2 text-lg font-semibold text-white/40">
                  USDC
                </span>
              </div>

              {numAmount > 0 && (
                <p className="mt-4 text-sm text-white/50">
                  In 1 year:{" "}
                  <span className="text-white font-medium tabular-nums">
                    {(numAmount * Math.exp(DOL_APY)).toFixed(2)} Dol
                  </span>
                </p>
              )}

              {/* Preset amount chips — the single biggest friction reducer
                  for a first-time BTC-lite user. Tapping any chip fills
                  the input; $1 is deliberately first to disarm the
                  "how much should I risk" anxiety. Each chip uses
                  `leading-none` + explicit `h-9` so mixed font weights
                  don't throw the baseline the way the earlier Max pill
                  did before we fixed it. */}
              <div className="mt-5 flex flex-wrap items-center gap-2">
                {[1, 10, 100, 500].map((preset) => {
                  const active = numAmount === preset;
                  const disabled =
                    usdcBalance !== null && preset > usdcBalance;
                  return (
                    <button
                      key={preset}
                      type="button"
                      onClick={() => setAmount(String(preset))}
                      disabled={disabled}
                      className={`inline-flex h-9 items-center rounded-full border px-4 text-[12px] font-semibold leading-none tabular-nums transition-colors ${
                        disabled
                          ? "cursor-not-allowed border-white/5 text-white/20"
                          : active
                            ? "border-white bg-white text-black"
                            : "border-white/15 text-white/75 hover:border-white/35 hover:text-white"
                      }`}
                    >
                      ${preset}
                    </button>
                  );
                })}
                {usdcBalance !== null && usdcBalance > 0 && (
                  <button
                    type="button"
                    onClick={() => setAmount(usdcBalance.toString())}
                    className="inline-flex h-9 items-center rounded-full border border-white/15 px-4 text-[11px] font-semibold uppercase leading-none tracking-wider text-white/75 transition-colors hover:border-white/35 hover:text-white"
                  >
                    Max
                  </button>
                )}
              </div>

              {/* Network fee hint — Phase 1 is on Base Sepolia testnet
                  so the effective fee is a fraction of a cent. We
                  surface this in a tiny line with a glossary tooltip
                  so a first-time user doesn't panic about hidden
                  costs. */}
              <p className="mt-4 flex items-center gap-1.5 text-[11px] text-white/35">
                <Glossary term="gas">Network fee</Glossary>
                <span>fractions of a cent on testnet</span>
              </p>

              {/*
                Phase 1 TVL capacity strip — published as a hard $100k
                ceiling in /docs/trust/framework-assumptions and now
                actually enforced. We surface a small live readout
                inside the amount card (below the network fee hint),
                and a louder banner appears between the card and the
                action button when the user's amount would exceed the
                remaining capacity. The banner is informational tone,
                not a red error — capacity is a normal product
                constraint, not a user mistake.
              */}
              {currentTvlUsdc !== null && (
                <div className="mt-4">
                  <div className="flex items-center justify-between text-[11px] text-white/35">
                    <span>Phase 1 capacity</span>
                    <span className="tabular-nums">
                      $
                      {currentTvlUsdc.toLocaleString("en-US", {
                        maximumFractionDigits: 0,
                      })}{" "}
                      / $
                      {PHASE_1_TVL_CAP_USDC.toLocaleString("en-US")}
                    </span>
                  </div>
                  <div
                    className="mt-2 h-[3px] w-full overflow-hidden rounded-full bg-white/[0.05]"
                    aria-hidden
                  >
                    <div
                      className={`h-full transition-all ${
                        tvlCapReached
                          ? "bg-amber-400/70"
                          : "bg-white/40"
                      }`}
                      style={{
                        width: `${Math.min(
                          100,
                          (currentTvlUsdc / PHASE_1_TVL_CAP_USDC) * 100,
                        )}%`,
                      }}
                    />
                  </div>
                </div>
              )}
            </div>

            {/* Warnings / errors */}
            {tvlCapReached && (
              <div className="mt-4 rounded-2xl border border-amber-400/20 bg-amber-400/[0.04] px-5 py-4 text-center">
                <p className="text-[13px] font-semibold text-amber-200">
                  Phase 1 capacity reached
                </p>
                <p className="mt-1 text-[11px] leading-relaxed text-amber-200/70">
                  Dol is currently at its $
                  {PHASE_1_TVL_CAP_USDC.toLocaleString("en-US")} TVL ceiling
                  for Phase 1. New deposits open up automatically as
                  existing depositors cash out, or as the protocol moves
                  into the next phase.
                </p>
              </div>
            )}
            {!tvlCapReached && amountExceedsCap && tvlRemaining !== null && (
              <div className="mt-4 rounded-2xl border border-amber-400/20 bg-amber-400/[0.04] px-5 py-4">
                <p className="text-[12px] leading-relaxed text-amber-200/85">
                  Only{" "}
                  <span className="font-semibold tabular-nums">
                    $
                    {tvlRemaining.toLocaleString("en-US", {
                      maximumFractionDigits: 2,
                    })}
                  </span>{" "}
                  of Phase 1 capacity is available right now. Lower the
                  amount, or come back after others cash out.
                </p>
              </div>
            )}
            {insufficientBalance && (
              <p className="mt-4 flex items-center gap-1.5 text-sm text-red-400">
                <AlertTriangle className="h-3.5 w-3.5" />
                Not enough balance.
              </p>
            )}

            {/* Inline faucet hint after a balance-related failure */}
            {lastErrorCategory === "insufficient_usdc" && (
              <a
                href={BASE_SEPOLIA_USDC_FAUCET}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-3 flex items-center justify-center gap-1.5 text-xs text-white/50 hover:text-white transition-colors"
              >
                Get test USDC from faucet &rarr;
              </a>
            )}
            {lastErrorCategory === "insufficient_eth" && (
              <a
                href={BASE_SEPOLIA_ETH_FAUCET}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-3 flex items-center justify-center gap-1.5 text-xs text-white/50 hover:text-white transition-colors"
              >
                Get a little test ETH for gas &rarr;
              </a>
            )}

            {/* Action button */}
            <div className="mt-6">
              {!authenticated ? (
                <button
                  onClick={login}
                  className="w-full rounded-full bg-white py-4 text-[17px] font-medium text-black hover:bg-white/90 transition-colors"
                >
                  Connect to buy
                </button>
              ) : needsApproval && !inputInvalid && !isApproveConfirmed ? (
                <button
                  onClick={handleApprove}
                  disabled={
                    isApproving ||
                    insufficientBalance ||
                    isWrongChain ||
                    tvlCapReached ||
                    amountExceedsCap
                  }
                  className="w-full rounded-full bg-white py-4 text-[17px] font-medium text-black hover:bg-white/90 transition-colors disabled:bg-white/10 disabled:text-white/40 disabled:cursor-not-allowed"
                >
                  {isApproving ? (
                    <>
                      <Loader2 className="inline mr-2 h-5 w-5 animate-spin" />
                      Setting up your first Dol...
                    </>
                  ) : isApproveConfirmed ? (
                    "All set. Tap to buy."
                  ) : tvlCapReached ? (
                    "Capacity reached"
                  ) : (
                    "Get my first Dol"
                  )}
                </button>
              ) : (
                <button
                  onClick={handleDeposit}
                  disabled={
                    inputInvalid ||
                    insufficientBalance ||
                    isDepositing ||
                    isWrongChain ||
                    tvlCapReached ||
                    amountExceedsCap
                  }
                  className="w-full rounded-full bg-white py-4 text-[17px] font-medium text-black hover:bg-white/90 transition-colors disabled:bg-white/10 disabled:text-white/40 disabled:cursor-not-allowed"
                >
                  {isDepositing ? (
                    <>
                      <Loader2 className="inline mr-2 h-5 w-5 animate-spin" />
                      Buying your Dol...
                    </>
                  ) : tvlCapReached ? (
                    "Capacity reached"
                  ) : (
                    "Buy a Dol"
                  )}
                </button>
              )}
            </div>

            <p className="mt-6 text-center text-[11px] text-white/30">
              Safe &middot; Instant &middot; Cash out anytime
            </p>

            {/* Verify-before-signing hint — a passive one-liner.
                No modal, no click, no friction. Sits below the CTA
                as a permanent reference so a user whose wallet prompt
                shows a DIFFERENT address has a direct comparison
                point without having to go anywhere. Mitigates
                malicious browser extensions that swap the to-address
                between our React code and the wallet request. */}
            {authenticated && (
              <p className="mt-3 text-center text-[10px] leading-relaxed text-white/25">
                Your wallet will ask you to sign on{" "}
                <span className="font-mono text-white/40">
                  {target.address.slice(0, 6)}…
                  {target.address.slice(-4)}
                </span>
                . Verify it matches before you approve.
              </p>
            )}
          </>
        )}
      </div>
      {tosModal}
      <SiteFooter />
    </main>
  );
}

export default function DepositPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-screen bg-black flex items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-white/40" />
        </div>
      }
    >
      <DepositPageInner />
    </Suspense>
  );
}
