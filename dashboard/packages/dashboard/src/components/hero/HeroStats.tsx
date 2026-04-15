"use client";

import { StatusPill } from "./StatusPill";
import { Skeleton } from "@/components/ui/skeleton";
import { AnimatedNumber } from "@/components/common/AnimatedNumber";
import { formatPct, formatSharePrice } from "@/lib/format";

/**
 * Hero stats row — Aurora-Ω Week 1 reality.
 *
 * The old V1.5 two-tier story (70% perp + 30% Moonwell) is gone.
 * Current product is a pure funding-capture engine across a 10-symbol
 * universe (7 crypto + 3 RWA), so the cards now surface:
 *
 *   1. Customer APY cap     — mandate ceiling (v3.5.2 production target)
 *   2. Gross engine APY     — live sim aggregate APY, or production gross
 *   3. Portfolio NAV        — aggregate $ from sim (or on-chain if deployed)
 *   4. Share price          — 1:1 at mint, drifts with NAV/supply
 *   5. Universe             — "10 · 7 crypto + 3 RWA"
 *   6. Status               — Aurora engine state
 */

export type HeroData = {
  customerApyCap: number;       // live customer APY (pre-cap or capped)
  customerCapped: boolean;      // true if customer value is the hard cap
  grossEngineApy: number;       // live gross engine APY on deployed capital
  grossApySource: "live" | "target";
  portfolioNavUsd: number;      // $ value of aggregate NAV
  sharePrice: number;           // e.g. 1.0000
  universeSize: number;         // 10
  universeBreakdown: string;    // "7 crypto + 3 RWA"
  status: "live" | "paused" | "offline";
};

type HeroStatsProps =
  | { state: "loaded"; data: HeroData }
  | { state: "loading" }
  | { state: "error"; message: string };

export function HeroStats(props: HeroStatsProps) {
  if (props.state === "loading") {
    return <HeroSkeleton />;
  }

  if (props.state === "error") {
    return (
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        {[...Array(6)].map((_, i) => (
          <div
            key={i}
            className="rounded-2xl border border-carry-red/30 bg-dark-surface p-5 text-center"
          >
            <p className="text-xs text-carry-red">{props.message}</p>
          </div>
        ))}
      </div>
    );
  }

  const {
    customerApyCap,
    customerCapped,
    grossEngineApy,
    grossApySource,
    portfolioNavUsd,
    sharePrice,
    universeSize,
    universeBreakdown,
    status,
  } = props.data;

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
      {/* 1. Customer APY — live from Aurora universe, capped at 8% mandate */}
      <div className="rounded-2xl border border-senior/30 bg-dark-surface p-5 text-center transition-colors hover:border-senior/50">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-senior">
          Customer APY
        </p>
        <p className="mt-2 font-mono text-[36px] font-semibold leading-none tracking-[-0.02em] text-senior">
          <AnimatedNumber value={customerApyCap} format={formatPct} />
        </p>
        <p className="mt-1 text-[11px] text-dark-secondary">
          {customerCapped ? "capped · 8% mandate" : "live · uncapped"}
        </p>
      </div>

      {/* 2. Gross engine APY */}
      <StatCard
        label="Gross APY"
        value={grossEngineApy}
        format={formatPct}
        sub={grossApySource === "live" ? "live · Aurora-Ω" : "v3.5.2 backtest"}
      />

      {/* 3. Portfolio NAV */}
      <StatCard
        label="Portfolio NAV"
        value={portfolioNavUsd}
        format={(v) =>
          `$${v.toLocaleString("en-US", {
            minimumFractionDigits: 0,
            maximumFractionDigits: 0,
          })}`
        }
        sub="10-pair aggregate"
      />

      {/* 4. Share Price */}
      <StatCard
        label="Share Price"
        value={sharePrice}
        format={formatSharePrice}
        sub="1 Dol ⇌ 1 USDC seed"
      />

      {/* 5. Universe */}
      <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 text-center transition-colors hover:border-dark-border-strong">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Universe
        </p>
        <p className="mt-2 font-mono text-[36px] font-semibold leading-none tracking-[-0.02em] text-dark-primary">
          {universeSize}
        </p>
        <p className="mt-1 text-[11px] text-dark-secondary">
          {universeBreakdown}
        </p>
      </div>

      {/* 6. Status */}
      <div className="flex flex-col items-center justify-center rounded-2xl border border-dark-border bg-dark-surface p-5 transition-colors hover:border-dark-border-strong">
        <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
          Status
        </p>
        <div className="mt-2">
          <StatusPill status={status} />
        </div>
        <p className="mt-1 text-[11px] text-dark-secondary">
          {status === "live" ? "Aurora-Ω · dry-run" : "engine offline"}
        </p>
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  format,
  sub,
}: {
  label: string;
  value: number;
  format: (n: number) => string;
  sub?: string;
}) {
  return (
    <div className="rounded-2xl border border-dark-border bg-dark-surface p-5 text-center transition-colors hover:border-dark-border-strong">
      <p className="text-[12px] font-medium uppercase tracking-[0.06em] text-dark-secondary">
        {label}
      </p>
      <p className="mt-2 font-mono text-[36px] font-semibold leading-none tracking-[-0.02em] text-dark-primary">
        <AnimatedNumber value={value} format={format} />
      </p>
      {sub && <p className="mt-1 text-[11px] text-dark-secondary">{sub}</p>}
    </div>
  );
}

function HeroSkeleton() {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
      {[...Array(6)].map((_, i) => (
        <div
          key={i}
          className="rounded-2xl border border-dark-border bg-dark-surface p-5 text-center"
        >
          <Skeleton className="mx-auto h-3 w-16 bg-dark-surface-2" />
          <Skeleton className="mx-auto mt-3 h-8 w-20 bg-dark-surface-2" />
        </div>
      ))}
    </div>
  );
}
