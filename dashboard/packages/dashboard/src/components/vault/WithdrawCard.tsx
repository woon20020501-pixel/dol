"use client";

import { useState, useEffect } from "react";
import { VaultCardSkeleton } from "@/components/common/LoadingSkeleton";
import { useWithdraw, type PendingRequest } from "@/hooks/useWithdraw";
import { useVaultReads } from "@/hooks/useVaultReads";
import { usePrivy } from "@privy-io/react-auth";
import { formatUsd } from "@/lib/format";
import { DEMO_VAULT } from "@/lib/demoData";
import { toast } from "sonner";
import {
  AlertTriangle,
  ArrowRightLeft,
  CheckCircle2,
  Clock,
  Info,
  Loader2,
} from "lucide-react";

export function WithdrawCard() {
  const { authenticated, login, ready } = usePrivy();
  const vault = useVaultReads();
  const wd = useWithdraw();
  const [amount, setAmount] = useState("");

  const userShares =
    vault.deployed && vault.userShares !== null
      ? vault.userShares
      : DEMO_VAULT.userShares;
  const sharePrice =
    vault.deployed && vault.sharePrice !== null
      ? vault.sharePrice
      : DEMO_VAULT.sharePrice;

  const numAmount = Number(amount) || 0;
  const estimatedUsdc = numAmount > 0 ? numAmount * sharePrice : 0;
  const exceedsBalance = numAmount > userShares;

  useEffect(() => {
    if (wd.isRequestConfirmed) {
      toast.success("Withdraw requested!", {
        description: `${amount} shares queued. Claimable in 24 hours.`,
      });
      setAmount("");
      wd.reset();
    }
  }, [wd.isRequestConfirmed]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (wd.isClaimConfirmed) {
      toast.success("Withdraw claimed!", {
        description: "USDC has been sent to your wallet.",
      });
      wd.reset();
    }
  }, [wd.isClaimConfirmed]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!ready) {
    return <VaultCardSkeleton />;
  }

  function renderRequestAction() {
    if (!authenticated) {
      return (
        <button onClick={login} className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-primary transition-colors hover:border-dark-border-strong">
          Connect Wallet
        </button>
      );
    }

    if (wd.isDemoMode) {
      return (
        <div className="space-y-1.5">
          <button disabled className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-tertiary cursor-not-allowed">
            Request Withdraw
          </button>
          <p className="flex items-center justify-center gap-1 text-[11px] text-dark-tertiary">
            <Info className="h-3 w-3" />
            Demo mode — transactions disabled
          </p>
        </div>
      );
    }

    if (!wd.deployed) {
      return (
        <div className="space-y-1.5">
          <button disabled className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-tertiary cursor-not-allowed">
            Request Withdraw
          </button>
          <p className="flex items-center justify-center gap-1 text-[11px] text-carry-amber">
            <AlertTriangle className="h-3 w-3" />
            Contract not deployed yet
          </p>
        </div>
      );
    }

    if (wd.isWrongNetwork) {
      return (
        <button onClick={wd.switchNetwork} className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-primary transition-colors hover:border-dark-border-strong">
          <ArrowRightLeft className="mr-2 inline h-4 w-4" />
          Switch to Base Sepolia
        </button>
      );
    }

    if (wd.isRequesting) {
      return (
        <button disabled className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-tertiary cursor-wait">
          <Loader2 className="mr-2 inline h-4 w-4 animate-spin" />
          Requesting...
        </button>
      );
    }

    const inputInvalid = !amount || numAmount <= 0;

    return (
      <button
        disabled={inputInvalid || exceedsBalance}
        onClick={() => wd.requestWithdraw(numAmount)}
        className="w-full rounded-xl border border-dark-border bg-dark-surface-2 py-3 text-[14px] font-medium text-dark-primary transition-colors hover:border-dark-border-strong disabled:text-dark-tertiary disabled:cursor-not-allowed"
      >
        Request Withdraw
      </button>
    );
  }

  const errorMsg = wd.requestError || wd.claimError;

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
      <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary mb-4">
        Withdraw
      </p>

      <div className="space-y-3">
        <div>
          <div className="flex items-center justify-between">
            <label
              htmlFor="withdraw-amount"
              className="text-[11px] uppercase tracking-[0.06em] text-dark-tertiary"
            >
              Shares
            </label>
            {authenticated && (
              <button
                type="button"
                onClick={() => setAmount(userShares.toString())}
                className="text-[11px] text-senior hover:text-senior-dark transition-colors rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-senior"
              >
                MAX: {userShares.toLocaleString()}
              </button>
            )}
          </div>
          <input
            id="withdraw-amount"
            type="number"
            min="0"
            step="0.01"
            placeholder="0.00"
            value={amount}
            onChange={(e) => {
              setAmount(e.target.value);
              if (errorMsg) wd.reset();
            }}
            className="mt-1 w-full rounded-xl border border-dark-border bg-dark-surface-2 px-4 py-2.5 text-[14px] font-mono text-dark-primary placeholder:text-dark-tertiary focus:outline-none focus:border-senior focus:ring-1 focus:ring-senior transition-colors"
            aria-label="Shares to withdraw"
          />
        </div>

        <div className="flex items-center justify-between rounded-xl bg-dark-surface-2 px-4 py-2.5 text-[12px]">
          <span className="text-dark-secondary">You receive (est.)</span>
          <span className="font-mono text-dark-primary">
            {formatUsd(estimatedUsdc)} USDC
          </span>
        </div>

        {exceedsBalance && (
          <p className="flex items-center gap-1 text-[11px] text-carry-red">
            <AlertTriangle className="h-3 w-3" />
            Exceeds your share balance
          </p>
        )}

        {errorMsg && (
          <div role="alert" className="flex items-start gap-1.5 rounded-xl bg-carry-red/10 border border-carry-red/20 px-3 py-2 text-[11px] text-carry-red">
            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
            <div className="flex-1">
              {wd.isReverted ? (
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
                  onClick={wd.reset}
                  className="underline hover:no-underline rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-senior"
                >
                  Try again
                </button>
                {wd.revertedHash && (
                  <a
                    href={`https://sepolia.basescan.org/tx/${wd.revertedHash}`}
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

        {renderRequestAction()}

        {wd.pendingRequests.length > 0 && (
          <div className="space-y-2 border-t border-dark-border pt-3">
            <h4 className="text-[11px] uppercase tracking-[0.06em] text-dark-secondary">
              Pending Requests
            </h4>
            {wd.pendingRequests.map((req) => (
              <PendingRequestRow
                key={req.requestId}
                req={req}
                isClaiming={wd.isClaiming && wd.claimingId === req.requestId}
                isClaimable={wd.isClaimable(req)}
                cooldownRemaining={wd.cooldownRemaining(req)}
                onClaim={() => wd.claimWithdraw(req.requestId)}
                disabled={wd.isDemoMode || !wd.deployed}
              />
            ))}
          </div>
        )}

        <p className="text-[11px] text-dark-tertiary text-center">
          24h cooldown after request before claiming
        </p>
      </div>
    </div>
  );
}

function PendingRequestRow({
  req,
  isClaiming,
  isClaimable,
  cooldownRemaining,
  onClaim,
  disabled,
}: {
  req: PendingRequest;
  isClaiming: boolean;
  isClaimable: boolean;
  cooldownRemaining: number;
  onClaim: () => void;
  disabled: boolean;
}) {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (isClaimable) return;
    const id = setInterval(() => setTick((t) => t + 1), 60_000);
    return () => clearInterval(id);
  }, [isClaimable]);

  return (
    <div className="flex items-center justify-between rounded-xl border border-dark-border bg-dark-surface-2 px-4 py-2.5">
      <div className="space-y-0.5">
        <p className="text-[13px] font-mono text-dark-primary">
          {req.shares.toLocaleString()} shares
        </p>
        {isClaimable ? (
          <p className="flex items-center gap-1 text-[11px] text-carry-green">
            <CheckCircle2 className="h-3 w-3" />
            Ready to claim
          </p>
        ) : (
          <p className="flex items-center gap-1 text-[11px] text-dark-tertiary">
            <Clock className="h-3 w-3" />
            {formatCountdown(cooldownRemaining)}
          </p>
        )}
      </div>
      {isClaiming ? (
        <button disabled className="rounded-lg border border-dark-border bg-dark-surface px-3 py-1.5 text-[12px] text-dark-tertiary cursor-wait">
          <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
          Claiming
        </button>
      ) : (
        <button
          disabled={!isClaimable || disabled}
          onClick={onClaim}
          className={`rounded-lg px-3 py-1.5 text-[12px] font-medium transition-colors ${
            isClaimable
              ? "bg-senior text-dark-bg hover:bg-senior-dark"
              : "border border-dark-border bg-dark-surface text-dark-tertiary cursor-not-allowed"
          }`}
        >
          Claim
        </button>
      )}
    </div>
  );
}

function formatCountdown(ms: number): string {
  const hours = Math.floor(ms / (1000 * 60 * 60));
  const minutes = Math.floor((ms % (1000 * 60 * 60)) / (1000 * 60));
  if (hours > 0) return `Claimable in ${hours}h ${minutes}m`;
  if (minutes > 0) return `Claimable in ${minutes}m`;
  return "Claimable soon";
}
