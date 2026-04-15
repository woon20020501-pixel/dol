"use client";

import { HeroStats, type HeroData } from "@/components/hero/HeroStats";
import { DepositCard } from "@/components/vault/DepositCard";
import { WithdrawCard } from "@/components/vault/WithdrawCard";
import { NavReporterCard } from "@/components/vault/NavReporterCard";
import { OfflineBanner } from "@/components/common/OfflineBanner";
import { ConnectButton } from "@/components/hero/ConnectButton";
import { ContractBanner } from "@/components/common/ContractBanner";
import { useVaultReads } from "@/hooks/useVaultReads";
import { useBotStatus } from "@/hooks/useBotStatus";
import { useBotHealth } from "@/hooks/useBotHealth";
import { useNavReporter } from "@/hooks/useNavReporter";
import { AuroraConsole } from "@/components/aurora/AuroraConsole";
import { MultiSymbolNavPanel } from "@/components/aurora/MultiSymbolNavPanel";
import {
  AURORA_CONSTANTS,
  SYMBOL_UNIVERSE,
  AuroraTelemetryProvider,
  useAuroraTelemetry,
} from "@/hooks/useAuroraTelemetry";

// Mandate constants from the v3.5.2 dry run.
// v3.5.2 production: gross 14.20% APY → customer 8.00% (capped),
// buffer 4.78%, reserve 1.42%. Treat 8/14.2 ≈ 0.5634 as the
// "customer share of gross before the cap kicks in".
const CUSTOMER_APY_HARD_CAP = 8.0;
const CUSTOMER_SHARE_OF_GROSS = 8.0 / 14.2;

/**
 * Steady-state expected gross APY computed from the SYMBOL_UNIVERSE
 * spread table — used as a stable fallback when the bot hasn't run
 * long enough for the rolling-rate estimate to be meaningful.
 */
function computeUniverseGrossApy(): number {
  const totalSpread = SYMBOL_UNIVERSE.reduce(
    (sum, s) => sum + s.spreadPct,
    0,
  );
  const totalIncomeUsdPerYear =
    (totalSpread / 100) * AURORA_CONSTANTS.pairNotionalUsd;
  return (
    (totalIncomeUsdPerYear / AURORA_CONSTANTS.deployedUsdDemo) * 100
  );
}

export default function Home() {
  // Single telemetry instance for the entire dashboard subtree. All
  // descendants read the shared state via useAuroraTelemetry() — no
  // more triplicate polling/derivation across AuroraConsole +
  // MultiSymbolNavPanel + page-level hero data.
  return (
    <AuroraTelemetryProvider>
      <DashboardBody />
    </AuroraTelemetryProvider>
  );
}

function DashboardBody() {
  const vault = useVaultReads();
  const botStatus = useBotStatus();
  const botHealth = useBotHealth();
  const navReporter = useNavReporter();
  const aurora = useAuroraTelemetry();

  // Status pill: bot health > paused > live
  const statusPillValue: "live" | "paused" | "offline" = botHealth.isOffline
    ? "offline"
    : botStatus.data?.paused
      ? "paused"
      : "live";

  // Hero data priority: when Aurora is in LIVE/STALE mode, ALWAYS prefer
  // the live aggregate over on-chain testnet reads (the testnet vault has
  // stale test deposits that have nothing to do with the bot's narrative).
  // SIM mode falls back to on-chain reads if the contract is deployed,
  // otherwise to the simulator output.
  const isAuroraLive = aurora.dataSource === "LIVE" || aurora.dataSource === "STALE";

  const portfolioNavUsd = isAuroraLive
    ? aurora.aggregate.navUsd
    : vault.deployed && vault.totalAssets !== null
      ? vault.totalAssets
      : aurora.aggregate.navUsd;

  const sharePrice = isAuroraLive
    ? aurora.aggregate.navUsd / AURORA_CONSTANTS.startingNavUsd
    : vault.deployed && vault.sharePrice !== null
      ? vault.sharePrice
      : aurora.aggregate.navUsd / AURORA_CONSTANTS.startingNavUsd;

  const rwaCount = SYMBOL_UNIVERSE.filter((s) => s.kind === "rwa").length;
  const cryptoCount = SYMBOL_UNIVERSE.length - rwaCount;

  // Gross APY derivation:
  //   - LIVE mode with enough sim time: rolling rate from the bot's
  //     aggregate cumulative pnl over its sim-elapsed window
  //   - LIVE mode with too little data: fall back to universe spread
  //   - SIM mode: universe spread (the sim implements exactly that rate)
  const universeGrossApy = computeUniverseGrossApy();
  const liveElapsedHours = aurora.simElapsedHours;
  const livePnlUsd = aurora.aggregate.pnlUsd;
  const liveRollingApy =
    isAuroraLive && liveElapsedHours > 0.5
      ? (livePnlUsd / AURORA_CONSTANTS.deployedUsdDemo) *
        (8760 / liveElapsedHours) *
        100
      : universeGrossApy;
  const grossEngineApy = Number.isFinite(liveRollingApy) ? liveRollingApy : universeGrossApy;

  const customerPreCap = grossEngineApy * CUSTOMER_SHARE_OF_GROSS;
  const liveCustomerApy = Math.max(
    0,
    Math.min(customerPreCap, CUSTOMER_APY_HARD_CAP),
  );
  const customerCapped = customerPreCap >= CUSTOMER_APY_HARD_CAP;

  const heroData: HeroData = {
    customerApyCap: liveCustomerApy,
    customerCapped,
    grossEngineApy,
    grossApySource: isAuroraLive ? "live" : "target",
    portfolioNavUsd,
    sharePrice,
    universeSize: SYMBOL_UNIVERSE.length,
    universeBreakdown: `${cryptoCount} crypto + ${rwaCount} RWA`,
    status: statusPillValue,
  };

  const decision = botStatus.data?.decision;


  return (
    <div className="relative min-h-screen overflow-x-hidden">
      {/* Subtle top glow — Apple Pro style */}
      <div className="pointer-events-none absolute left-1/2 top-0 -translate-x-1/2 h-[400px] w-[min(1000px,100vw)] bg-senior/[0.04] rounded-full blur-[180px]" />

      <main className="relative z-10 mx-auto max-w-[1440px] px-4 py-4 sm:px-8 sm:py-6">
        {/* Header row */}
        <header className="mb-6 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <span className="text-[18px] font-semibold tracking-[-0.01em] text-dark-primary">
              Dol
            </span>
            {decision ? (
              <span className="hidden rounded-full border border-dark-border bg-dark-surface px-2.5 py-0.5 font-mono text-[11px] text-senior sm:inline-block">
                {decision.action}
              </span>
            ) : null}
            {navReporter.status === "error" ? (
              <span
                className="hidden items-center gap-1 text-[11px] text-carry-red sm:inline-flex"
                role="status"
                aria-label="NAV reporter is reporting errors"
              >
                <span
                  className="inline-block h-1.5 w-1.5 rounded-full bg-carry-red"
                  aria-hidden="true"
                />
                reporter error
              </span>
            ) : null}
            {navReporter.status === "never" &&
            heroData.portfolioNavUsd > 0 &&
            !navReporter.isLoading ? (
              <span
                className="hidden items-center gap-1 text-[11px] text-carry-amber sm:inline-flex"
                role="status"
                aria-label="Awaiting first NAV report"
              >
                <span
                  className="inline-block h-1.5 w-1.5 rounded-full bg-carry-amber"
                  aria-hidden="true"
                />
                awaiting first NAV report
              </span>
            ) : null}
          </div>
          <div className="flex items-center gap-4">
            <a
              href="/"
              className="hidden text-[13px] text-dark-secondary transition-colors hover:text-dark-primary sm:block"
            >
              &larr; Landing
            </a>
            <ConnectButton />
          </div>
        </header>

        {/* Contract not deployed banner */}
        <ContractBanner deployed={vault.deployed} />

        {/* Offline banner */}
        {botHealth.isOffline && (
          <div className="mb-4">
            <OfflineBanner visible />
          </div>
        )}

        {/* Aurora-Ω Week-1 demo console — JARVIS-style operate layer */}
        <AuroraConsole />

        {/* Hero stats row */}
        <section aria-label="Key metrics" className="mb-4">
          {botStatus.isLoading ? (
            <HeroStats state="loading" />
          ) : (
            <HeroStats
              state="loaded"
              data={heroData}
            />
          )}
        </section>

        {/* Main grid: multi-symbol telemetry on left, vault actions on right */}
        <div className="grid gap-4 lg:grid-cols-[1fr_340px]">
          <div className="space-y-4">
            <MultiSymbolNavPanel />
          </div>

          {/* Right column: deposit, withdraw, NAV reporter trust card */}
          <aside className="space-y-4">
            <DepositCard />
            <WithdrawCard />
            <NavReporterCard />
          </aside>
        </div>

        {/* Footer */}
        <footer className="mt-6 flex flex-wrap items-center justify-between gap-2 border-t border-dark-border pt-4 font-mono text-[11px] text-dark-secondary">
          <span>Dol · Operator · Aurora-Ω Week 1</span>
          <span>
            Universe{" "}
            <span className="text-dark-primary">
              {AURORA_CONSTANTS.universeSizeDemo}
            </span>
            <span className="text-dark-tertiary">/{AURORA_CONSTANTS.universeSizeProd}</span>
            {" · "}BTC spread{" "}
            <span className="text-senior">
              +{AURORA_CONSTANTS.spreadAnnualBps}bps
            </span>
            {" · "}dry-run
          </span>
        </footer>
      </main>
    </div>
  );
}

