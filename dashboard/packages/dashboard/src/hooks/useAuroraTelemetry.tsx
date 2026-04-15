"use client";

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

// Authoritative numbers from the Week-1 runtime fixture (2026-04-15).
// Per-tick NAV should come from nav.jsonl once the runtime is wired;
// for now we drive a deterministic accel-factor simulator client-side so the
// dashboard can present the Week-1 story even without the Rust bot running.

export const AURORA_CONSTANTS = {
  startingNavUsd: 10_000,
  pairNotionalUsd: 100,
  entryCostUsdNav: 0.10,        // per-pair step down at t=0 (NAV level)
  year1NetIncomeUsd: 18.82,     // single-pair NAV-level net
  year1GrossBps: 18.92,
  year1GrossUsd: 18.92,
  breakevenHours: 46.3,         // BTC single-pair breakeven
  spreadAnnualPct: 18.9158,
  spreadAnnualBps: 1892,
  pacificaFundingPct: -13.8058,
  backpackFundingPct: 5.1100,
  tickIntervalSec: 10,
  cycleLengthSec: 3600,         // Pacifica 1-hour funding cycle
  accelFactor: 3600,            // 1 real sec = 1 simulated hour
  longVenue: "Pacifica",
  shortVenue: "Backpack",
  symbol: "BTC",
  // multi-symbol universe
  universeSizeDemo: 10,
  universeSizeProd: 46,
  deployedUsdDemo: 1_000,       // 10 × $100
  deployedFractionDemo: 0.10,
  deployedFractionProd: 0.50,
  // Chart static x-window. The interesting dynamics (entry cost step,
  // breakeven crossings for all pairs) happen within ~100 sim hours.
  // Keeping this fixed means the curve shapes are computed once and
  // stay stable regardless of how long the demo has been running — a
  // moving time marker is layered on top for liveness.
  chartWindowHours: 120,
};

export type SymbolKind = "crypto" | "rwa";

export type SymbolSpec = {
  symbol: string;
  spreadPct: number;        // annualized spread APY (abs value used for sim)
  color: string;
  tier: "anomaly" | "strong" | "mid" | "tail";
  kind: SymbolKind;
  counterVenue: string;
  oracleDivergenceRisk: "minimal" | "structural";
  // Static baseline for diagnostics.book_parse_failures.
  // RWA symbols (XAU/XAG/PAXG) are degraded-by-construction because
  // Lighter and Backpack don't list gold/silver commodities; the bot
  // routes them through Pacifica ↔ Hyperliquid (xyz:GOLD/SILVER).
  // This is expected operational state, NOT a fault, and must render
  // as a gray "reduced coverage" badge (never red alarm).
  bookParseDegraded: boolean;
};

// Canonical 10-symbol universe: 7 crypto + 3 RWA.
// Spreads are median values from week1_hist_spreads.json. BTC is
// overridden to the Week-1 Pacifica live anomaly (18.92%); the
// historical median is 0.39% — BTC is the "live outlier" demo case.
// XAU/XAG/PAXG carry oracle_divergence_risk = "structural".
export const SYMBOL_UNIVERSE: SymbolSpec[] = [
  { symbol: "BTC",  spreadPct: 18.92, color: "#f7931a", tier: "anomaly", kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "XAG",  spreadPct: 12.50, color: "#c0c0c8", tier: "strong",  kind: "rwa",    counterVenue: "trade.xyz:SILVER", oracleDivergenceRisk: "structural", bookParseDegraded: true  },
  { symbol: "XAU",  spreadPct: 12.20, color: "#ffd34e", tier: "strong",  kind: "rwa",    counterVenue: "trade.xyz:GOLD",   oracleDivergenceRisk: "structural", bookParseDegraded: true  },
  { symbol: "AVAX", spreadPct: 11.74, color: "#e84142", tier: "strong",  kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "PAXG", spreadPct: 10.89, color: "#f5b45e", tier: "strong",  kind: "rwa",    counterVenue: "Hyperliquid",      oracleDivergenceRisk: "structural", bookParseDegraded: true  },
  { symbol: "BNB",  spreadPct:  9.45, color: "#f3ba2f", tier: "mid",     kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "ARB",  spreadPct:  9.05, color: "#28a0f0", tier: "mid",     kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "SUI",  spreadPct:  8.14, color: "#4da2ff", tier: "mid",     kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "SOL",  spreadPct:  5.77, color: "#9945ff", tier: "tail",    kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
  { symbol: "ETH",  spreadPct:  4.48, color: "#627eea", tier: "tail",    kind: "crypto", counterVenue: "Hyperliquid",      oracleDivergenceRisk: "minimal",    bookParseDegraded: false },
];

export type SymbolState = {
  symbol: string;
  color: string;
  tier: SymbolSpec["tier"];
  kind: SymbolKind;
  counterVenue: string;
  oracleDivergenceRisk: "minimal" | "structural";
  bookParseDegraded: boolean;
  spreadPct: number;
  navPair: number;        // notional-level pair NAV (starts at $100)
  pnlPair: number;        // net USD gain on the pair (can be negative)
  pnlBps: number;         // bps vs notional
  series: { t: number; nav: number }[];
  rank: number;           // 1 = top gain right now
};

export type Venue = "Pacifica" | "Hyperliquid" | "Lighter" | "Backpack";

export type VenueHealth = {
  venue: Venue;
  status: "live" | "fixture" | "offline";
  label: string;
  fundingApyPct: number | null;
  ageSec: number;
  // In LIVE mode we count how many of the 10 signal JSONs include this
  // venue under fair_value.contributing_venues.
  // null in SIM mode (signal data unavailable).
  symbolCoverage: { covered: number; total: number } | null;
};

export type DecisionEvent = {
  id: number;
  tsSimMs: number;
  kind: "open" | "hold" | "cycle_lock" | "rebalance" | "stub";
  message: string;
};

export type RiskLayer = {
  name: string;
  label: string;
  red: boolean;
  stub: boolean;
};

export type AggregateState = {
  navUsd: number;              // portfolio NAV across all 10 pairs
  pnlUsd: number;              // net vs $10k seed
  pnlBps: number;
  series: { t: number; nav: number }[];
  breakevenHours: number;      // when aggregate pnl = 0
  breakevenReached: boolean;
  hoursToBreakeven: number;
  activePairs: number;
};

export type DataSource = "LIVE" | "STALE" | "SIM";

export type AuroraTelemetry = {
  dataSource: DataSource;
  liveFileMtimeMs: number | null;
  liveLatestTsMs: number | null;
  liveFilePath: string | null;
  liveSignalSource: DataSource;                 // independent of nav dataSource
  liveSignalReceivedAtMs: number | null;        // for "Ns ago" age display
  simElapsedHours: number;
  simMs: number;
  navSeries: { t: number; nav: number }[];   // t in sim hours (BTC single-pair for legacy use)
  navUsd: number;
  cumAccrualUsd: number;
  lastIncomeUsd: number;
  lastCostUsd: number;
  breakevenReached: boolean;
  hoursToBreakeven: number;
  positionEvent: "Idle" | "Opened" | "Held" | "Rebalanced";
  cycleLock: {
    locked: boolean;
    cycleIndex: number;
    hC: "long" | "short";
    nC: number;
    secondsToCycleEnd: number;
    cycleProgress: number;
    proposedWasBlocked: boolean;
  };
  pairDecision: {
    longVenue: Venue;
    shortVenue: Venue;
    spreadAnnualPct: number;
    notionalUsd: number;
    reason: string;
  };
  venues: VenueHealth[];
  riskStack: RiskLayer[];
  stubbedSections: string[];
  decisions: DecisionEvent[];
  fsmMode: "nominal" | "derisk" | "flatten";
  fsmNotionalScale: number;
  symbols: SymbolState[];       // 10-symbol state, re-ranked every tick
  aggregate: AggregateState;    // portfolio-level aggregate (thick line)
};

const STORAGE_KEY = "aurora-telemetry-anchor-v1";

function getAnchor(): number {
  if (typeof window === "undefined") return Date.now();
  const stored = sessionStorage.getItem(STORAGE_KEY);
  if (stored) {
    const parsed = Number(stored);
    if (Number.isFinite(parsed)) return parsed;
  }
  const now = Date.now();
  sessionStorage.setItem(STORAGE_KEY, String(now));
  return now;
}

function computeNavAtSimHour(simHours: number): {
  nav: number;
  cumAccrual: number;
  lastIncome: number;
  lastCost: number;
  event: "Idle" | "Opened" | "Held";
} {
  const { startingNavUsd, entryCostUsdNav, year1GrossUsd } = AURORA_CONSTANTS;
  const accrualPerHour = year1GrossUsd / 8760;

  if (simHours <= 0) {
    return {
      nav: startingNavUsd,
      cumAccrual: 0,
      lastIncome: 0,
      lastCost: 0,
      event: "Idle",
    };
  }

  // At t=0+, entry cost step-down happens once.
  const income = simHours * accrualPerHour;
  const nav = startingNavUsd - entryCostUsdNav + income;
  return {
    nav,
    cumAccrual: income - entryCostUsdNav,
    lastIncome: accrualPerHour / 360, // per 100ms tick
    lastCost: simHours < 0.01 ? entryCostUsdNav : 0,
    event: simHours < 0.01 ? "Opened" : "Held",
  };
}

function buildVenues(simHours: number): VenueHealth[] {
  // SIM mode fixture values from the demo day snapshot. In LIVE mode the
  // per-venue rows below get rebuilt from /api/signal data, with symbol
  // coverage counts replacing the funding APY estimates.
  return [
    {
      venue: "Pacifica",
      status: "live",
      label: "LIVE · public REST",
      fundingApyPct: AURORA_CONSTANTS.pacificaFundingPct,
      ageSec: (simHours * 3600) % 8,
      symbolCoverage: null,
    },
    {
      venue: "Backpack",
      status: "fixture",
      label: "FIXTURE · Week 1",
      fundingApyPct: AURORA_CONSTANTS.backpackFundingPct,
      ageSec: 0,
      symbolCoverage: null,
    },
    {
      venue: "Hyperliquid",
      status: "fixture",
      label: "FIXTURE · Week 1",
      fundingApyPct: 2.85,
      ageSec: 0,
      symbolCoverage: null,
    },
    {
      venue: "Lighter",
      status: "fixture",
      label: "FIXTURE · Week 1",
      fundingApyPct: -0.9,
      ageSec: 0,
      symbolCoverage: null,
    },
  ];
}

const DECISION_SCRIPT: Omit<DecisionEvent, "id">[] = [
  { tsSimMs: 0, kind: "stub", message: "fsm: nominal · notional_scale=1.00" },
  { tsSimMs: 50, kind: "open", message: "fair_value · p_star computed from 1 venue (Pacifica live)" },
  { tsSimMs: 200, kind: "open", message: "pair_decision · LONG Pacifica / SHORT Backpack · spread 1892 bps" },
  { tsSimMs: 400, kind: "open", message: "order · would_have_executed · notional $100.00 · reason=spread_threshold" },
  { tsSimMs: 800, kind: "cycle_lock", message: "cycle_lock: ENGAGED · c=0 · h_c=long · N_c=$100" },
  { tsSimMs: 1400, kind: "hold", message: "tick · holding position · accrual +$0.002/hr" },
  { tsSimMs: 2200, kind: "hold", message: "risk_stack: 4/4 green (ECV, χ², RL, entropic)" },
  { tsSimMs: 3100, kind: "cycle_lock", message: "cycle_lock: proposal=flip · BLOCKED · h_c locked until c=1" },
  { tsSimMs: 4200, kind: "hold", message: "fair_value healthy=true · contributing_venues=1" },
];

function buildDecisions(simMs: number, maxLines = 8): DecisionEvent[] {
  const out: DecisionEvent[] = [];
  for (let i = 0; i < DECISION_SCRIPT.length; i++) {
    const scripted = DECISION_SCRIPT[i];
    if (scripted.tsSimMs <= simMs) {
      out.push({ id: i, ...scripted });
    }
  }
  // Also add synthetic heartbeat lines every ~1500ms so the log keeps moving.
  const heartbeats = Math.floor(simMs / 1500);
  for (let h = 0; h < heartbeats; h++) {
    const hbTs = h * 1500 + 500;
    if (hbTs <= simMs && hbTs > (DECISION_SCRIPT[DECISION_SCRIPT.length - 1]?.tsSimMs ?? 0)) {
      out.push({
        id: 1000 + h,
        tsSimMs: hbTs,
        kind: "hold",
        message: `tick · nav=${(AURORA_CONSTANTS.startingNavUsd - AURORA_CONSTANTS.entryCostUsdNav + (hbTs / 1000 / 3600) * (AURORA_CONSTANTS.year1GrossUsd / 8760) * AURORA_CONSTANTS.accelFactor).toFixed(4)}`,
      });
    }
  }
  return out.slice(-maxLines).reverse();
}

function computeSymbolNav(spec: SymbolSpec, simHours: number): number {
  // Per-pair NAV starts at $100 notional, loses $0.10 entry cost once,
  // then accrues income at spread_pct of notional per year.
  const { pairNotionalUsd, entryCostUsdNav } = AURORA_CONSTANTS;
  if (simHours <= 0) return pairNotionalUsd;
  const incomePerHour = (spec.spreadPct / 100) * pairNotionalUsd / 8760;
  return pairNotionalUsd - entryCostUsdNav + simHours * incomePerHour;
}

function buildSymbolSeries(
  spec: SymbolSpec,
  _currentSimHours: number,
  points = 80,
): { t: number; nav: number }[] {
  // Fixed 0..chartWindowHours grid — curve shapes are stable across ticks,
  // so memoization upstream makes this a one-time cost after mount.
  const maxT = AURORA_CONSTANTS.chartWindowHours;
  const out: { t: number; nav: number }[] = [];
  for (let i = 0; i <= points; i++) {
    const t = (i / points) * maxT;
    out.push({ t, nav: computeSymbolNav(spec, t) });
  }
  return out;
}

function buildAggregate(simHours: number): AggregateState {
  const { startingNavUsd, pairNotionalUsd, entryCostUsdNav, chartWindowHours } =
    AURORA_CONSTANTS;
  const totalSpread = SYMBOL_UNIVERSE.reduce((sum, s) => sum + s.spreadPct, 0);
  const totalIncomePerHour =
    (totalSpread / 100) * pairNotionalUsd / 8760;
  const totalEntryCost = SYMBOL_UNIVERSE.length * entryCostUsdNav;

  const navFor = (t: number) =>
    t <= 0 ? startingNavUsd : startingNavUsd - totalEntryCost + t * totalIncomePerHour;

  const nav = navFor(simHours);
  const pnlUsd = nav - startingNavUsd;
  const pnlBps = (pnlUsd / startingNavUsd) * 10_000;
  const breakevenHours = totalEntryCost / totalIncomePerHour;

  // Static chart window — curve shape stops evolving after mount so the
  // chart doesn't thrash as sim time advances.
  const points = 100;
  const series: { t: number; nav: number }[] = [];
  for (let i = 0; i <= points; i++) {
    const t = (i / points) * chartWindowHours;
    series.push({ t, nav: navFor(t) });
  }

  return {
    navUsd: nav,
    pnlUsd,
    pnlBps,
    series,
    breakevenHours,
    breakevenReached: simHours >= breakevenHours,
    hoursToBreakeven: Math.max(0, breakevenHours - simHours),
    activePairs: SYMBOL_UNIVERSE.length,
  };
}

function buildSymbolStates(simHours: number): SymbolState[] {
  const states: SymbolState[] = SYMBOL_UNIVERSE.map((spec) => {
    const nav = computeSymbolNav(spec, simHours);
    const pnl = nav - AURORA_CONSTANTS.pairNotionalUsd;
    return {
      symbol: spec.symbol,
      color: spec.color,
      tier: spec.tier,
      kind: spec.kind,
      counterVenue: spec.counterVenue,
      oracleDivergenceRisk: spec.oracleDivergenceRisk,
      bookParseDegraded: spec.bookParseDegraded,
      spreadPct: spec.spreadPct,
      navPair: nav,
      pnlPair: pnl,
      pnlBps: (pnl / AURORA_CONSTANTS.pairNotionalUsd) * 10_000,
      series: buildSymbolSeries(spec, simHours),
      rank: 0,
    };
  });
  const sorted = [...states].sort((a, b) => b.pnlPair - a.pnlPair);
  sorted.forEach((s, i) => {
    const idx = states.findIndex((x) => x.symbol === s.symbol);
    states[idx].rank = i + 1;
  });
  return states;
}

// ──────────────────────────────────────────────────────────────────────────
// Live nav.jsonl path
// ──────────────────────────────────────────────────────────────────────────

type NavRow = {
  ts_ms: number;
  symbol: string;
  nav_usd: number;
  cumulative_accrual_usd?: number;
  delta_usd?: number;
  last_income_usd?: number;
  last_cost_usd?: number;
  position_event?: string;
  event?: string;
};

type LiveSymbol = {
  series: { t: number; nav: number }[];    // t = (ts_ms - firstTs) / 3_600_000 (hours)
  navUsd: number;
  cumAccrualUsd: number;
  lastTsMs: number;
  event: string;
};

type LiveState = {
  firstTsMs: number;
  latestTsMs: number;
  aggregate: LiveSymbol | null;
  symbols: Record<string, LiveSymbol>;
  fileMtimeMs: number;
  filePath: string;
  isStale: boolean;
};

const LIVE_SERIES_MAX_POINTS = 2_000;

function emptyLiveSymbol(): LiveSymbol {
  return {
    series: [],
    navUsd: 0,
    cumAccrualUsd: 0,
    lastTsMs: 0,
    event: "",
  };
}

function mergeLiveRows(prev: LiveState | null, rows: NavRow[], meta: {
  fileMtimeMs: number;
  filePath: string;
  isStale: boolean;
}): LiveState {
  // Seed from prev if we have it; otherwise start fresh.
  const state: LiveState = prev
    ? {
        firstTsMs: prev.firstTsMs,
        latestTsMs: prev.latestTsMs,
        aggregate: prev.aggregate ? { ...prev.aggregate, series: prev.aggregate.series.slice() } : null,
        symbols: Object.fromEntries(
          Object.entries(prev.symbols).map(([k, v]) => [k, { ...v, series: v.series.slice() }]),
        ),
        fileMtimeMs: meta.fileMtimeMs,
        filePath: meta.filePath,
        isStale: meta.isStale,
      }
    : {
        firstTsMs: 0,
        latestTsMs: 0,
        aggregate: null,
        symbols: {},
        fileMtimeMs: meta.fileMtimeMs,
        filePath: meta.filePath,
        isStale: meta.isStale,
      };

  for (const row of rows) {
    if (typeof row.ts_ms !== "number" || typeof row.nav_usd !== "number") continue;
    if (state.firstTsMs === 0) state.firstTsMs = row.ts_ms;
    if (row.ts_ms > state.latestTsMs) state.latestTsMs = row.ts_ms;

    const tHours = (row.ts_ms - state.firstTsMs) / 3_600_000;

    if (row.symbol === "AGGREGATE") {
      // The bot writes the real portfolio NAV here ($10k base + accruals).
      if (!state.aggregate) state.aggregate = emptyLiveSymbol();
      const agg = state.aggregate;
      agg.series.push({ t: tHours, nav: row.nav_usd });
      if (agg.series.length > LIVE_SERIES_MAX_POINTS) {
        agg.series.splice(0, agg.series.length - LIVE_SERIES_MAX_POINTS);
      }
      agg.navUsd = row.nav_usd;
      agg.cumAccrualUsd = row.cumulative_accrual_usd ?? 0;
      agg.lastTsMs = row.ts_ms;
      agg.event = row.event ?? row.position_event ?? "";
    } else {
      // Per-symbol rows: the bot's `nav_usd` is on the $10k portfolio
      // scale (each symbol reports the global NAV from its perspective),
      // NOT on the $100 notional scale. The actual per-pair gain lives
      // in `cumulative_accrual_usd`. Reconstruct a notional-scale nav
      // = $100 + cum_accrual so the sparkline tile + per-symbol chart
      // line stay on the same $100-base axis as the simulator path.
      if (!state.symbols[row.symbol]) state.symbols[row.symbol] = emptyLiveSymbol();
      const sym = state.symbols[row.symbol];
      const cumAccrual = row.cumulative_accrual_usd ?? 0;
      const navAtPairScale = AURORA_CONSTANTS.pairNotionalUsd + cumAccrual;
      sym.series.push({ t: tHours, nav: navAtPairScale });
      if (sym.series.length > LIVE_SERIES_MAX_POINTS) {
        sym.series.splice(0, sym.series.length - LIVE_SERIES_MAX_POINTS);
      }
      sym.navUsd = navAtPairScale;
      sym.cumAccrualUsd = cumAccrual;
      sym.lastTsMs = row.ts_ms;
      sym.event = row.event ?? row.position_event ?? "";
    }
  }

  return state;
}

type NavApiResponse =
  | {
      ok: true;
      rows: NavRow[];
      file_path: string;
      file_mtime_ms: number;
      latest_symbol_ts_ms: number;
      is_stale: boolean;
    }
  | {
      ok: false;
      error: string;
      fallback_to_simulator: true;
    };

function useLiveNav(): {
  dataSource: DataSource;
  state: LiveState | null;
} {
  const [dataSource, setDataSource] = useState<DataSource>("SIM");
  const [state, setState] = useState<LiveState | null>(null);
  const stateRef = useRef<LiveState | null>(null);
  const lastTsSeenRef = useRef<number>(0);

  useEffect(() => {
    let cancelled = false;

    async function pollOnce() {
      try {
        const qs = lastTsSeenRef.current > 0
          ? `?since_ms=${lastTsSeenRef.current}`
          : "";
        const res = await fetch(`/api/nav${qs}`, { cache: "no-store" });
        if (!res.ok) throw new Error(`/api/nav ${res.status}`);
        const data = (await res.json()) as NavApiResponse;
        if (cancelled) return;

        if (!data.ok) {
          setDataSource("SIM");
          setState(null);
          stateRef.current = null;
          lastTsSeenRef.current = 0;
          return;
        }

        const merged = mergeLiveRows(stateRef.current, data.rows, {
          fileMtimeMs: data.file_mtime_ms,
          filePath: data.file_path,
          isStale: data.is_stale,
        });
        stateRef.current = merged;
        setState(merged);
        lastTsSeenRef.current = merged.latestTsMs;
        setDataSource(data.is_stale ? "STALE" : "LIVE");
      } catch {
        if (cancelled) return;
        setDataSource("SIM");
        setState(null);
        stateRef.current = null;
        lastTsSeenRef.current = 0;
      }
    }

    pollOnce();
    const interval = setInterval(pollOnce, 2_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return { dataSource, state };
}

// ──────────────────────────────────────────────────────────────────────────
// Live signal JSON (per-symbol)
// ──────────────────────────────────────────────────────────────────────────

type SignalPairDecision = {
  long_venue: string;
  short_venue: string;
  symbol: string;
  spread_annual: number;        // fraction (0.17 = 17%)
  notional_usd: number;
  reason: string;
  would_have_executed: boolean;
};

type SignalCycleLock = {
  locked: boolean;
  cycle_index: number;
  h_c: number;                  // 1 = long, -1 = short
  N_c: number;
  seconds_to_cycle_end: number;
  proposed_was_blocked: boolean;
  emergency_override: boolean;
  opened_new_cycle: boolean;
};

type SignalDoc = {
  symbol: string;
  ts_unix: number;
  fair_value: {
    p_star: number;
    contributing_venues: string[];
    healthy: boolean;
  };
  cycle_lock: SignalCycleLock;
  fsm: {
    mode: string;
    notional_scale: number;
    emergency_flatten: boolean;
    _stub?: boolean;
  };
  risk_stack: Array<{
    layer: string;
    red_flag: boolean;
    _stub?: boolean;
  }>;
  diagnostics: {
    stubbed_sections: string[];
    pacifica_authenticated?: boolean;
    book_parse_failures?: {
      is_degraded: boolean;
      consecutive_failures: number;
      last_failure_venue: string | null;
    };
  };
  extra: {
    pair_decision: SignalPairDecision;
  };
};

type SignalApiResponse =
  | {
      ok: true;
      signals: Record<string, SignalDoc>;
      n_symbols: number;
      file_mtime_ms_max: number;
      is_stale: boolean;
      signals_root: string;
    }
  | {
      ok: false;
      error: string;
      fallback_to_simulator: true;
    };

type LiveSignalState = {
  signals: Record<string, SignalDoc>;
  fileMtimeMsMax: number;
  isStale: boolean;
  receivedAtMs: number;      // wall clock when the dashboard received this snapshot
};

function useLiveSignal(): {
  source: DataSource;
  state: LiveSignalState | null;
} {
  const [source, setSource] = useState<DataSource>("SIM");
  const [state, setState] = useState<LiveSignalState | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function pollOnce() {
      try {
        const res = await fetch("/api/signal", { cache: "no-store" });
        if (!res.ok) throw new Error(`/api/signal ${res.status}`);
        const data = (await res.json()) as SignalApiResponse;
        if (cancelled) return;
        if (!data.ok) {
          setSource("SIM");
          setState(null);
          return;
        }
        setState({
          signals: data.signals,
          fileMtimeMsMax: data.file_mtime_ms_max,
          isStale: data.is_stale,
          receivedAtMs: Date.now(),
        });
        setSource(data.is_stale ? "STALE" : "LIVE");
      } catch {
        if (cancelled) return;
        setSource("SIM");
        setState(null);
      }
    }

    pollOnce();
    // Signal JSONs change slowly relative to nav.jsonl — 5s is enough.
    const interval = setInterval(pollOnce, 5_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return { source, state };
}

// Internal hook: runs the full telemetry pipeline ONCE inside the provider.
// All consumers read through React Context instead of re-invoking this hook,
// which eliminates triplicate work (3 components × full build per frame).
function useAuroraTelemetrySource(): AuroraTelemetry {
  const { dataSource, state: live } = useLiveNav();
  const { source: signalSource, state: liveSignal } = useLiveSignal();
  const anchorRef = useRef<number | null>(null);
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    anchorRef.current = getAnchor();
    let raf = 0;
    // Throttle React re-renders to 60fps. rAF keeps firing at the native
    // refresh rate (120/144/165Hz) but we only bump state every ~16ms.
    // Smooth to the eye, caps the derivation work at 60× per second
    // instead of 144× × consumers.
    let lastSetMs = 0;
    const THROTTLE_MS = 16;
    const tick = () => {
      const now = performance.now();
      if (now - lastSetMs >= THROTTLE_MS) {
        lastSetMs = now;
        setFrame((n) => (n + 1) & 0x3fffffff);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  const anchor = anchorRef.current ?? Date.now();
  const realMs = Math.max(0, Date.now() - anchor);
  // Accel 3600×: 1 real second = 1 simulated hour.
  const simHours = realMs / 1000;
  const simDisplayMs = simHours * 3600 * 1000;

  // Cheap per-frame values: NAV counter, cycle ring, venue ages.
  // Expensive series: recompute at 5Hz — quantize to 0.2 sim hours.
  const seriesBucket = Math.floor(simHours * 5) / 5;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const aggregate = useMemo(() => buildAggregate(seriesBucket), [seriesBucket]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const symbolStates = useMemo(
    () => buildSymbolStates(seriesBucket),
    [seriesBucket],
  );
  // re-rank on every frame using fresh sim time so counters stay live
  const liveSymbols = symbolStates.map((s) => {
    const spec = SYMBOL_UNIVERSE.find((u) => u.symbol === s.symbol)!;
    const nav = computeSymbolNav(spec, simHours);
    return {
      ...s,
      navPair: nav,
      pnlPair: nav - AURORA_CONSTANTS.pairNotionalUsd,
      pnlBps: ((nav - AURORA_CONSTANTS.pairNotionalUsd) / AURORA_CONSTANTS.pairNotionalUsd) * 10_000,
    };
  });
  const sortedLive = [...liveSymbols].sort((a, b) => b.pnlPair - a.pnlPair);
  sortedLive.forEach((s, i) => {
    const idx = liveSymbols.findIndex((x) => x.symbol === s.symbol);
    liveSymbols[idx].rank = i + 1;
  });

  // Reference `frame` to guarantee we react every animation frame even
  // though the derived values are computed imperatively above.
  void frame;

  // Legacy single-pair (BTC) quantities retained for the Aurora pair card.
  const { cumAccrual, lastIncome, lastCost, event } =
    computeNavAtSimHour(simHours);

  const cycleSecondsElapsed = (simHours * 3600) % AURORA_CONSTANTS.cycleLengthSec;
  const secondsToCycleEnd = AURORA_CONSTANTS.cycleLengthSec - cycleSecondsElapsed;
  const cycleProgress = cycleSecondsElapsed / AURORA_CONSTANTS.cycleLengthSec;
  const cycleIndex = Math.floor((simHours * 3600) / AURORA_CONSTANTS.cycleLengthSec);

  const riskStack: RiskLayer[] = [
    { name: "entropic_ce",  label: "Entropic CE",   red: false, stub: true  },
    { name: "ecv",          label: "ECV (CVaR₉₉)",  red: false, stub: true  },
    { name: "exec_chi2",    label: "Exec χ²",       red: false, stub: true  },
    { name: "rl_critic",    label: "RL Critic",     red: false, stub: true  },
  ];

  // When live data is available, OVERRIDE the simulator's aggregate and
  // per-symbol state with what the bot actually wrote. Everything
  // else (cycleLock, pairDecision, venues, riskStack, decisions, breakeven
  // reference values) stays on the sim — those don't come from nav.jsonl.
  const useLive = (dataSource === "LIVE" || dataSource === "STALE") && live !== null;

  const finalAggregate: AggregateState = (() => {
    if (!useLive || !live.aggregate) return aggregate;
    const navUsd = live.aggregate.navUsd;
    const pnlUsd = navUsd - AURORA_CONSTANTS.startingNavUsd;
    const pnlBps = (pnlUsd / AURORA_CONSTANTS.startingNavUsd) * 10_000;
    return {
      navUsd,
      pnlUsd,
      pnlBps,
      series: live.aggregate.series.map((p) => ({ t: p.t, nav: p.nav })),
      breakevenHours: aggregate.breakevenHours,
      breakevenReached: pnlUsd >= 0,
      hoursToBreakeven: Math.max(0, aggregate.breakevenHours - (live.latestTsMs - live.firstTsMs) / 3_600_000),
      activePairs: Object.keys(live.symbols).length || SYMBOL_UNIVERSE.length,
    };
  })();

  const finalSymbols: SymbolState[] = (() => {
    if (!useLive) return liveSymbols;
    const merged: SymbolState[] = SYMBOL_UNIVERSE.map((spec) => {
      const liveSym = live.symbols[spec.symbol];
      if (!liveSym) {
        return {
          symbol: spec.symbol,
          color: spec.color,
          tier: spec.tier,
          kind: spec.kind,
          counterVenue: spec.counterVenue,
          oracleDivergenceRisk: spec.oracleDivergenceRisk,
          bookParseDegraded: spec.bookParseDegraded,
          spreadPct: spec.spreadPct,
          navPair: AURORA_CONSTANTS.pairNotionalUsd,
          pnlPair: 0,
          pnlBps: 0,
          series: [],
          rank: 0,
        };
      }
      const nav = liveSym.navUsd;
      const pnl = nav - AURORA_CONSTANTS.pairNotionalUsd;
      return {
        symbol: spec.symbol,
        color: spec.color,
        tier: spec.tier,
        kind: spec.kind,
        counterVenue: spec.counterVenue,
        oracleDivergenceRisk: spec.oracleDivergenceRisk,
        bookParseDegraded: spec.bookParseDegraded,
        spreadPct: spec.spreadPct,
        navPair: nav,
        pnlPair: pnl,
        pnlBps: (pnl / AURORA_CONSTANTS.pairNotionalUsd) * 10_000,
        series: liveSym.series.map((p) => ({ t: p.t, nav: p.nav })),
        rank: 0,
      };
    });
    const sorted = [...merged].sort((a, b) => b.pnlPair - a.pnlPair);
    sorted.forEach((s, i) => {
      const idx = merged.findIndex((x) => x.symbol === s.symbol);
      merged[idx].rank = i + 1;
    });
    return merged;
  })();

  const finalSimElapsedHours = useLive && live
    ? Math.max(0, (live.latestTsMs - live.firstTsMs) / 3_600_000)
    : simHours;

  // ────── /api/signal overrides ──────
  // When liveSignal is available we OVERRIDE the per-venue cells, the BTC
  // pair_decision card, the cycle lock ring, fsm.mode badge, and
  // stubbedSections — none of those come from nav.jsonl.

  const haveLiveSignal = signalSource !== "SIM" && liveSignal !== null;
  const signalDocs = haveLiveSignal ? liveSignal!.signals : {};
  const btcSignal: SignalDoc | undefined = signalDocs.BTC;
  const symbolsWithSignal = Object.keys(signalDocs);

  // Build per-venue coverage: for each of the 4 venues, count how many
  // signal JSONs include it under fair_value.contributing_venues. RWA
  // symbols won't include Lighter or Backpack, so the count
  // naturally surfaces the structural reduced-coverage state.
  const liveVenues: VenueHealth[] = haveLiveSignal
    ? (["Pacifica", "Hyperliquid", "Lighter", "Backpack"] as Venue[]).map((v) => {
        let covered = 0;
        for (const sym of symbolsWithSignal) {
          const cv = signalDocs[sym]?.fair_value?.contributing_venues ?? [];
          if (cv.includes(v)) covered += 1;
        }
        const total = symbolsWithSignal.length || 10;
        // In LIVE mode every venue we know about is by definition reachable
        // from the bot's perspective — the count just shows how many of the
        // 10 symbols' fair_value happens to include it. RWA symbols don't
        // list Lighter/Backpack so those will show 7/10 by construction.
        // We never label this as "offline" — the bot's running, it just
        // isn't contributing for some symbols.
        return {
          venue: v,
          status: "live",
          label: `LIVE · fair_value contributor`,
          fundingApyPct: null,
          ageSec: 0,
          symbolCoverage: { covered, total },
        };
      })
    : buildVenues(simHours);

  const livePairDecision = haveLiveSignal && btcSignal
    ? {
        longVenue: btcSignal.extra.pair_decision.long_venue as Venue,
        shortVenue: btcSignal.extra.pair_decision.short_venue as Venue,
        spreadAnnualPct: btcSignal.extra.pair_decision.spread_annual * 100,
        notionalUsd: btcSignal.extra.pair_decision.notional_usd,
        reason: btcSignal.extra.pair_decision.reason,
      }
    : {
        longVenue: "Pacifica" as Venue,
        shortVenue: "Backpack" as Venue,
        spreadAnnualPct: AURORA_CONSTANTS.spreadAnnualPct,
        notionalUsd: AURORA_CONSTANTS.pairNotionalUsd,
        reason: "spread_threshold_met",
      };

  const liveCycleLock = haveLiveSignal && btcSignal
    ? {
        locked: btcSignal.cycle_lock.locked,
        cycleIndex: btcSignal.cycle_lock.cycle_index,
        hC: (btcSignal.cycle_lock.h_c >= 0 ? "long" : "short") as "long" | "short",
        nC: btcSignal.cycle_lock.N_c,
        secondsToCycleEnd: btcSignal.cycle_lock.seconds_to_cycle_end,
        cycleProgress:
          1 - btcSignal.cycle_lock.seconds_to_cycle_end / AURORA_CONSTANTS.cycleLengthSec,
        proposedWasBlocked: btcSignal.cycle_lock.proposed_was_blocked,
      }
    : {
        locked: simHours > 0.05,
        cycleIndex,
        hC: "long" as const,
        nC: AURORA_CONSTANTS.pairNotionalUsd,
        secondsToCycleEnd,
        cycleProgress,
        proposedWasBlocked: simHours > 0.8,
      };

  const liveFsmMode: "nominal" | "derisk" | "flatten" =
    haveLiveSignal && btcSignal
      ? btcSignal.fsm.mode === "kelly_safe" || btcSignal.fsm.mode === "nominal"
        ? "nominal"
        : btcSignal.fsm.mode.includes("flatten")
          ? "flatten"
          : "derisk"
      : "nominal";

  const liveFsmNotionalScale =
    haveLiveSignal && btcSignal ? btcSignal.fsm.notional_scale : 1.0;

  const liveStubbedSections =
    haveLiveSignal && btcSignal
      ? btcSignal.diagnostics.stubbed_sections
      : ["forecast_scoring", "risk_stack", "fsm"];

  return {
    dataSource,
    liveFileMtimeMs: live?.fileMtimeMs ?? null,
    liveLatestTsMs: live?.latestTsMs ?? null,
    liveFilePath: live?.filePath ?? null,
    liveSignalSource: signalSource,
    liveSignalReceivedAtMs: liveSignal?.receivedAtMs ?? null,
    simElapsedHours: finalSimElapsedHours,
    simMs: simDisplayMs,
    navSeries: finalAggregate.series,
    navUsd: finalAggregate.navUsd,
    cumAccrualUsd: cumAccrual,
    lastIncomeUsd: lastIncome,
    lastCostUsd: lastCost,
    breakevenReached: finalAggregate.breakevenReached,
    hoursToBreakeven: finalAggregate.hoursToBreakeven,
    positionEvent: event,
    cycleLock: liveCycleLock,
    pairDecision: livePairDecision,
    venues: liveVenues,
    riskStack,
    stubbedSections: liveStubbedSections,
    decisions: buildDecisions(realMs),
    fsmMode: liveFsmMode,
    fsmNotionalScale: liveFsmNotionalScale,
    symbols: finalSymbols,
    aggregate: finalAggregate,
  };
}

// ──────────────────────────────────────────────────────────────────────────
// Single-instance provider + context
// ──────────────────────────────────────────────────────────────────────────

const AuroraContext = createContext<AuroraTelemetry | null>(null);

/**
 * Wraps the dashboard subtree and runs the Aurora telemetry pipeline
 * exactly ONCE. All descendant components read via useAuroraTelemetry()
 * from React Context — no duplicate polling, no duplicate derivations,
 * no out-of-sync re-render timelines.
 */
export function AuroraTelemetryProvider({ children }: { children: ReactNode }) {
  const value = useAuroraTelemetrySource();
  return (
    <AuroraContext.Provider value={value}>{children}</AuroraContext.Provider>
  );
}

/**
 * Public hook — reads the shared telemetry state from Context. Must be
 * called inside an <AuroraTelemetryProvider>.
 */
export function useAuroraTelemetry(): AuroraTelemetry {
  const ctx = useContext(AuroraContext);
  if (ctx === null) {
    throw new Error(
      "useAuroraTelemetry() must be used inside <AuroraTelemetryProvider>",
    );
  }
  return ctx;
}
