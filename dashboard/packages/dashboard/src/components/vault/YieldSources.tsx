"use client";

import { HelpCircle, Waves, Landmark } from "lucide-react";
import { formatPct } from "@/lib/format";

type YieldSourcesProps = {
  fundingApy: number;
  baseApy: number;
  marginShare: number;
  treasuryShare: number;
  treasuryConnected: boolean;
};

export function YieldSources({
  fundingApy,
  baseApy,
  marginShare,
  treasuryShare,
  treasuryConnected,
}: YieldSourcesProps) {
  const fundingWeighted = fundingApy * marginShare;
  const baseWeighted = baseApy * treasuryShare;

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <SourceCard
        accent="senior"
        icon={<Waves className="h-4 w-4 text-senior" aria-hidden="true" />}
        title="Funding Alpha"
        venue="Pacifica USDJPY"
        rawApy={fundingApy}
        weight={marginShare}
        weightedApy={fundingWeighted}
        line1="delta-neutral, hedged on Lighter"
        line2="captures perp funding rate spread"
        tooltip="Short Pacifica perp + long Lighter perp at the same notional. The vault collects Pacifica's funding payments while the price exposure cancels out across venues."
        liveBadge="live"
      />
      <SourceCard
        accent="junior"
        icon={<Landmark className="h-4 w-4 text-junior" aria-hidden="true" />}
        title="Treasury Yield"
        venue={treasuryConnected ? "Mock Moonwell Market" : "Mock Moonwell (sim)"}
        rawApy={baseApy}
        weight={treasuryShare}
        weightedApy={baseWeighted}
        line1="permissionless USDC lending"
        line2="\u2192 Moonwell at V2 mainnet"
        tooltip="30% of TVL is supplied to a Moonwell-style lending market for a low-risk base yield. Routes to real Moonwell on V2; simulated at 5% on testnet."
        liveBadge={treasuryConnected ? "live" : "sim"}
      />
    </div>
  );
}

type SourceCardProps = {
  accent: "senior" | "junior";
  icon: React.ReactNode;
  title: string;
  venue: string;
  rawApy: number;
  weight: number;
  weightedApy: number;
  line1: string;
  line2: string;
  tooltip: string;
  liveBadge: "live" | "sim";
};

function SourceCard({
  accent,
  icon,
  title,
  venue,
  rawApy,
  weight,
  weightedApy,
  line1,
  line2,
  tooltip,
  liveBadge,
}: SourceCardProps) {
  const borderColor =
    accent === "senior"
      ? "border-senior/30 hover:border-senior/50"
      : "border-junior/30 hover:border-junior/50";
  const accentColor = accent === "senior" ? "text-senior" : "text-junior";

  return (
    <div className={`rounded-2xl border bg-dark-surface p-5 transition-colors ${borderColor}`}>
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-2">
          {icon}
          <h3 className="text-[14px] font-semibold tracking-tight text-dark-primary">{title}</h3>
        </div>
        <div className="flex items-center gap-2">
          {/* Live badge */}
          <span className="inline-flex items-center gap-1">
            <span
              className={`inline-block h-1.5 w-1.5 rounded-full ${
                liveBadge === "live"
                  ? "bg-carry-green animate-pulse"
                  : "bg-carry-amber"
              }`}
              aria-hidden="true"
            />
            <span className="text-[10px] uppercase tracking-[0.06em] text-dark-secondary">
              {liveBadge === "live" ? "live" : "sim"}
            </span>
          </span>
          <InfoTooltip text={tooltip} />
        </div>
      </div>

      <p className="mt-1 text-[11px] text-dark-tertiary">{venue}</p>

      <div className="mt-3 flex items-baseline gap-2">
        <span className="font-mono text-[22px] font-semibold text-dark-primary">
          {formatPct(rawApy)}
        </span>
        <span className="text-[11px] text-dark-secondary">
          APY \u00d7 <span className="font-mono">{Math.round(weight * 100)}%</span> wgt
        </span>
      </div>
      <p className={`mt-0.5 font-mono text-[11px] ${accentColor}`}>
        \u2192 contributes {formatPct(weightedApy)} to total
      </p>

      <div className="mt-3 space-y-0.5 text-[11px] text-dark-secondary">
        <p>{line1}</p>
        <p>{line2}</p>
      </div>
    </div>
  );
}

function InfoTooltip({ text }: { text: string }) {
  return (
    <span className="group relative inline-flex">
      <button
        type="button"
        aria-label={text}
        title={text}
        className="rounded-full p-0.5 text-dark-tertiary transition-colors hover:text-dark-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-senior focus-visible:ring-offset-2 focus-visible:ring-offset-dark-bg"
      >
        <HelpCircle className="h-3.5 w-3.5" aria-hidden="true" />
      </button>
      <span
        role="tooltip"
        className="pointer-events-none absolute right-0 top-6 z-20 hidden w-64 rounded-xl border border-dark-border bg-dark-surface-2 p-3 text-[11px] leading-snug text-dark-primary shadow-lg group-hover:block group-focus-within:block"
      >
        {text}
      </span>
    </span>
  );
}
