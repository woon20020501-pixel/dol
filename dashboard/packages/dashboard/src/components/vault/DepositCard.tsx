"use client";

import { useState, useEffect } from "react";
import { VaultCardSkeleton } from "@/components/common/LoadingSkeleton";
import { useDeposit } from "@/hooks/useDeposit";
import { usePrivy } from "@privy-io/react-auth";
import { formatUsd } from "@/lib/format";
import { toast } from "sonner";
import {
  AlertTriangle,
  ArrowRightLeft,
  CheckCircle2,
  Loader2,
  Info,
} from "lucide-react";

export function DepositCard() {
  const { authenticated, login, ready } = usePrivy();
  const dep = useDeposit();
  const [amount, setAmount] = useState("");

  const numAmount = Number(amount) || 0;
  const hasInsufficientBalance =
    dep.usdcBalance !== null && numAmount > dep.usdcBalance;

  useEffect(() => {
    if (dep.isDepositConfirmed) {
      const depositedAmount = amount;
      const hash = dep.depositHash;
      toast.success("Deposit confirmed!", {
        description: `${depositedAmount} USDC deposited into the vault.`,
        action: hash
          ? {
              label: "View tx",
              onClick: () =>
                window.open(
                  `https://sepolia.basescan.org/tx/${hash}`,
                  "_blank",
                  "noopener,noreferrer",
                ),
            }
          : undefined,
        duration: 8_000,
      });
      setAmount("");
      dep.reset();
    }
  }, [dep.isDepositConfirmed]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!ready) {
    return <VaultCardSkeleton />;
  }

  function renderAction() {
    if (!authenticated) {
      return (
        <button onClick={login} className="w-full rounded-xl bg-senior py-3 text-[14px] font-medium text-dark-bg transition-colors hover:bg-senior-dark">
          Connect Wallet
        </button>
      );
    }

    if (dep.isDemoMode) {
      return (
        <div className="space-y-1.5">
          <button disabled className="w-full rounded-xl bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-tertiary cursor-not-allowed">
            Deposit
          </button>
          <p className="flex items-center justify-center gap-1 text-[11px] text-dark-tertiary">
            <Info className="h-3 w-3" />
            Demo mode — transactions disabled
          </p>
        </div>
      );
    }

    if (!dep.deployed) {
      return (
        <div className="space-y-1.5">
          <button disabled className="w-full rounded-xl bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-tertiary cursor-not-allowed">
            Deposit
          </button>
          <p className="flex items-center justify-center gap-1 text-[11px] text-carry-amber">
            <AlertTriangle className="h-3 w-3" />
            Contract not deployed yet
          </p>
        </div>
      );
    }

    if (dep.isWrongNetwork) {
      return (
        <button onClick={dep.switchNetwork} className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-primary transition-colors hover:border-dark-border-strong">
          <ArrowRightLeft className="mr-2 inline h-4 w-4" />
          Switch to Base Sepolia
        </button>
      );
    }

    if (dep.isApproving) {
      return (
        <button disabled className="w-full rounded-xl bg-senior/20 py-3 text-[14px] font-medium text-senior cursor-wait">
          <Loader2 className="mr-2 inline h-4 w-4 animate-spin" />
          Approving USDC...
        </button>
      );
    }

    // Bridge state: approval tx confirmed but allowance refetch still in
    // flight. Deposit is safe to press (approve-MAX guarantees allowance
    // >> deposit amount) but we show a distinct label so the user knows
    // something is happening instead of a confusing instant jump.
    if (dep.isRefetchingAllowance) {
      return (
        <button disabled className="w-full rounded-xl bg-senior/20 py-3 text-[14px] font-medium text-senior cursor-wait">
          <Loader2 className="mr-2 inline h-4 w-4 animate-spin" />
          Confirming allowance...
        </button>
      );
    }

    if (dep.isDepositing) {
      return (
        <button disabled className="w-full rounded-xl bg-senior/20 py-3 text-[14px] font-medium text-senior cursor-wait">
          <Loader2 className="mr-2 inline h-4 w-4 animate-spin" />
          Depositing...
        </button>
      );
    }

    if (dep.isDepositConfirmed) {
      return (
        <button disabled className="w-full rounded-xl bg-carry-green/20 py-3 text-[14px] font-medium text-carry-green">
          <CheckCircle2 className="mr-2 inline h-4 w-4" />
          Deposit Confirmed
        </button>
      );
    }

    const inputInvalid = !amount || numAmount <= 0;

    // After approval confirms we skip needsApproval() entirely — approve
    // was sent for maxUint256 so the bot's allowance is guaranteed to be
    // >> any deposit, even while refetchAllowance is still catching up.
    const shouldShowApprove =
      !inputInvalid && !dep.isApproveConfirmed && dep.needsApproval(numAmount);

    if (shouldShowApprove) {
      return (
        <button
          disabled={hasInsufficientBalance}
          onClick={() => dep.approve()}
          className="w-full rounded-xl bg-senior py-3 text-[14px] font-medium text-dark-bg transition-colors hover:bg-senior-dark disabled:bg-dark-surface-2 disabled:text-dark-tertiary disabled:cursor-not-allowed"
        >
          Approve USDC
        </button>
      );
    }

    return (
      <button
        disabled={inputInvalid || hasInsufficientBalance}
        onClick={() => dep.deposit(numAmount)}
        className="w-full rounded-xl bg-senior py-3 text-[14px] font-medium text-dark-bg transition-colors hover:bg-senior-dark disabled:bg-dark-surface-2 disabled:text-dark-tertiary disabled:cursor-not-allowed"
      >
        Deposit
      </button>
    );
  }

  const errorMsg = dep.approveError || dep.depositError;

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary mb-4">
        Deposit USDC
      </p>

      <div className="space-y-3">
        <div>
          <div className="flex items-center justify-between">
            <label
              htmlFor="deposit-amount"
              className="text-[11px] uppercase tracking-[0.06em] text-dark-secondary"
            >
              Amount
            </label>
            {authenticated && dep.usdcBalance !== null && (() => {
              // Capture usdcBalance in a local const so TS narrows it
              // to `number` inside the onClick closure without needing
              // a non-null assertion. Without this, the closure reads
              // the prop again at click time and TS widens it back.
              const bal = dep.usdcBalance;
              return (
                <button
                  type="button"
                  onClick={() => setAmount(bal.toString())}
                  className="text-[11px] text-senior hover:text-senior-dark transition-colors rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-senior"
                >
                  BAL: {formatUsd(bal)}
                </button>
              );
            })()}
          </div>
          <div className="relative mt-1">
            <input
              id="deposit-amount"
              // text (not number) so we can fully control what's typed or
              // pasted. inputMode="decimal" still pops the numeric keyboard
              // on phones. Sanitizer below strips non-digits / non-dot,
              // caps at one dot, truncates fractional part to 6 decimals
              // (USDC precision), and rejects scientific notation / NaN /
              // negatives — all of which an unguarded type="number" would
              // happily accept (e.g. pasting "1e18" typed a 10^18 deposit).
              type="text"
              inputMode="decimal"
              autoComplete="off"
              autoCorrect="off"
              spellCheck={false}
              placeholder="0.00"
              value={amount}
              onChange={(e) => {
                const raw = e.target.value;
                // Strip anything that isn't a digit or dot. This kills
                // "e", "E", "-", "+", commas, whitespace, everything else.
                let cleaned = raw.replace(/[^\d.]/g, "");
                // At most one dot.
                const firstDot = cleaned.indexOf(".");
                if (firstDot !== -1) {
                  cleaned =
                    cleaned.slice(0, firstDot + 1) +
                    cleaned.slice(firstDot + 1).replace(/\./g, "");
                  // Cap fractional part at 6 decimals (USDC).
                  const [whole, frac = ""] = cleaned.split(".");
                  cleaned = whole + "." + frac.slice(0, 6);
                }
                setAmount(cleaned);
                if (errorMsg) dep.reset();
              }}
              onKeyDown={(e) => {
                // Enter submits nothing useful here — eat it so the user
                // doesn't accidentally reload or submit a parent form.
                if (e.key === "Enter") e.preventDefault();
              }}
              className="w-full rounded-xl border border-dark-border bg-dark-surface-2 px-4 py-2.5 pr-16 text-[14px] font-mono text-dark-primary placeholder:text-white/30 transition-colors focus:border-senior focus:outline-none focus:ring-1 focus:ring-senior"
              aria-label="USDC amount to deposit"
            />
            <span className="absolute right-4 top-1/2 -translate-y-1/2 text-[12px] text-dark-secondary">
              USDC
            </span>
          </div>
        </div>

        {hasInsufficientBalance && (
          <p className="flex items-center gap-1 text-[11px] text-carry-red">
            <AlertTriangle className="h-3 w-3" />
            Insufficient USDC balance
          </p>
        )}

        {/* Pre-flight gas warning. Fires if the user's Base Sepolia ETH
            balance is below LOW_GAS_ETH (0.0005 ETH ≈ enough for 2
            standard txs with a 50× gas spike). Much better than
            watching MetaMask reject with "insufficient funds for gas"
            after the user already clicked Approve. */}
        {authenticated && dep.hasLowGas && (
          <div
            role="status"
            className="flex items-start gap-1.5 rounded-xl border border-carry-amber/30 bg-carry-amber/10 px-3 py-2 text-[11px] text-carry-amber"
          >
            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
            <div>
              <p>
                Low ETH for gas ({dep.ethBalance?.toFixed(5)} ETH). Approve
                and deposit both need a little more.
              </p>
              <a
                href="https://www.alchemy.com/faucets/base-sepolia"
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-block underline transition-opacity hover:no-underline"
              >
                Get test ETH &rarr;
              </a>
            </div>
          </div>
        )}

        {errorMsg && (
          <div role="alert" className="flex items-start gap-1.5 rounded-xl bg-carry-red/10 border border-carry-red/20 px-3 py-2 text-[11px] text-carry-red">
            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
            <div className="flex-1">
              {dep.isReverted ? (
                <>
                  <p className="font-medium">Reverted on-chain</p>
                  <p className="line-clamp-2 mt-0.5 text-dark-secondary">{errorMsg}</p>
                </>
              ) : (
                <p className="line-clamp-2">{errorMsg}</p>
              )}
              <div className="mt-1 flex items-center gap-2">
                <button
                  type="button"
                  onClick={dep.reset}
                  className="underline hover:no-underline rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-senior"
                >
                  Try again
                </button>
                {dep.revertedHash && (
                  <a
                    href={`https://sepolia.basescan.org/tx/${dep.revertedHash}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="underline hover:no-underline rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-senior"
                  >
                    View tx &rarr;
                  </a>
                )}
              </div>
            </div>
          </div>
        )}

        {renderAction()}
      </div>
    </div>
  );
}
