"use client";

import { Skeleton } from "@/components/ui/skeleton";
import { formatUsdCompact } from "@/lib/format";

type AllocationBarProps =
  | {
      state: "loaded";
      marginUsd: number;
      treasuryUsd: number;
      marginShare: number;
      treasuryShare: number;
      treasuryConnected: boolean;
    }
  | { state: "loading" }
  | { state: "empty" };

export function AllocationBar(props: AllocationBarProps) {
  if (props.state === "loading") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <Skeleton className="h-4 w-40 bg-dark-surface-2" />
        <Skeleton className="mt-3 h-12 w-full bg-dark-surface-2" />
      </div>
    );
  }

  if (props.state === "empty") {
    return (
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5">
        <div className="flex items-baseline justify-between">
          <h3 className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
            Capital Allocation
          </h3>
          <span className="text-[11px] text-dark-tertiary">
            awaiting first deposit
          </span>
        </div>
        <div className="mt-3 flex h-12 w-full overflow-hidden rounded-xl bg-dark-surface-2">
          <div className="flex h-full w-[70%] items-center justify-center bg-senior/20 text-[11px] font-medium text-senior">
            Margin 70%
          </div>
          <div className="flex h-full w-[30%] items-center justify-center bg-junior/20 text-[11px] font-medium text-junior">
            Treasury 30%
          </div>
        </div>
        <p className="mt-2 text-[11px] text-dark-tertiary">
          Target split — capital will route on the next deposit.
        </p>
      </div>
    );
  }

  const {
    marginUsd,
    treasuryUsd,
    marginShare,
    treasuryShare,
    treasuryConnected,
  } = props;

  const marginPct = Math.round(marginShare * 100);
  const treasuryPct = Math.round(treasuryShare * 100);

  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
      <div className="flex items-baseline justify-between">
        <h3 className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Capital Allocation
        </h3>
        <span className="text-[11px] text-dark-tertiary">
          {treasuryConnected ? "live on-chain" : "target split"}
        </span>
      </div>

      <div className="mt-3 flex h-12 w-full overflow-hidden rounded-xl bg-dark-surface-2">
        <div
          className="flex h-full items-center justify-center bg-gradient-to-r from-senior to-senior-dark text-[12px] font-semibold text-dark-bg transition-[width] duration-700"
          style={{ width: `${marginShare * 100}%` }}
          role="presentation"
        >
          {marginPct >= 18 ? `Margin ${marginPct}%` : null}
        </div>
        <div
          className="flex h-full items-center justify-center bg-gradient-to-r from-junior to-junior-dark text-[12px] font-semibold text-dark-bg transition-[width] duration-700"
          style={{ width: `${treasuryShare * 100}%` }}
          role="presentation"
        >
          {treasuryPct >= 12 ? `Treasury ${treasuryPct}%` : null}
        </div>
      </div>

      <div className="mt-3 flex items-center justify-between text-[12px] text-dark-secondary">
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-2.5 w-2.5 rounded bg-senior" />
          Margin&nbsp;
          <span className="font-mono text-dark-primary">{formatUsdCompact(marginUsd)}</span>
        </span>
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-2.5 w-2.5 rounded bg-junior" />
          Treasury&nbsp;
          <span className="font-mono text-dark-primary">{formatUsdCompact(treasuryUsd)}</span>
        </span>
      </div>
    </div>
  );
}
