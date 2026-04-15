/**
 * Deterministic demo data for the dashboard.
 * Numbers are consistent with PLAN.md section 2.3:
 * - Pacifica USDJPY funding: ~11.4% APY (stuck at +1.2 bps / 8h)
 * - Lighter USDJPY funding: ~-0.9% APY
 * - 30-day spread: ~12.3% APY
 * - Net APY to depositors after fees: ~7.5%
 * - TVL target: $10k–$50k range
 */

// ── Vault stats ──

export const DEMO_VAULT = {
  tvl: 28_450.0, // $28,450 USDC
  sharePrice: 1.0062, // slightly above 1.0 = vault has earned
  totalShares: 28_274.12,
  userShares: 5_000.0,
  userAssets: 5_031.0, // shares * sharePrice
  currentApyNet: 7.5, // net APY to depositors
  currentApyGross: 9.0,
} as const;

// ── Bot status ──

export const DEMO_STATUS = {
  nav: {
    value: "28450000000", // 28,450 USDC in 6 decimals
    timestamp: seedTimestamp(0),
    lastReportedOnChain: "28420000000",
  },
  positions: {
    pacifica: {
      symbol: "USDJPY",
      side: "SHORT" as const,
      notionalUsd: 6_200,
      entryPrice: 149.85,
      unrealizedPnl: 12.4,
    },
    hedge: {
      venue: "Lighter",
      symbol: "USDJPY",
      side: "LONG" as const,
      notionalUsd: 6_180,
      entryPrice: 149.82,
      unrealizedPnl: -8.2,
    },
  },
  fundingRates: {
    pacifica: {
      rate8h: 0.012, // +1.2 bps = 0.012%
      apyEquivalent: 13.14,
      nextTimestamp: seedTimestamp(0) + 8 * 3600,
    },
    lighter: {
      rate8h: -0.00082,
      apyEquivalent: -0.9,
      nextTimestamp: seedTimestamp(0) + 8 * 3600,
    },
  },
  decision: {
    action: "hold" as const,
    reason: "Carry spread +12.3% APY exceeds threshold. Maintaining position.",
  },
  carryScore: {
    // value is the net carry APY as a decimal fraction (0.1274 = 12.74%)
    // NOT a 0-1 quality score. Must match the real bot's output format.
    value: 0.1274,
    components: {
      funding: 0.1314,
      hedgeCost: -0.002,
      txCost: -0.002,
      riskPenalty: 0,
    },
  },
  dryRun: false,
  paused: false,
} as const;

// ── Health ──

export const DEMO_HEALTH = {
  ok: true,
  lastTickAge: 3,
  errors: [] as string[],
} as const;

// ── Funding spread chart (24h, one point per 30 min = 48 points) ──

export function generateFundingSpreadData() {
  const points = [];
  const now = seedTimestamp(0);
  const interval = 30 * 60; // 30 min

  for (let i = 47; i >= 0; i--) {
    const ts = now - i * interval;
    // Pacifica stuck at ~1.2 bps with tiny variance
    const pacificaRate = 1.2 + deterministicNoise(i, 0.15);
    // Lighter oscillates around -0.08 bps
    const lighterRate = -0.08 + deterministicNoise(i + 100, 0.25);
    points.push({
      timestamp: ts,
      time: formatTime(ts),
      pacifica: Number(pacificaRate.toFixed(3)),
      lighter: Number(lighterRate.toFixed(3)),
      spread: Number((pacificaRate - lighterRate).toFixed(3)),
    });
  }
  return points;
}

// ── Cumulative PnL chart (7 days, one point per hour = 168 points) ──

export function generateCumulativePnlData() {
  const points = [];
  const now = seedTimestamp(0);
  const interval = 3600; // 1 hour
  let cumulative = 0;

  for (let i = 167; i >= 0; i--) {
    const ts = now - i * interval;
    // ~$2.80/day funding earned on ~$6k notional at 17% gross spread
    // = $0.117/hour average
    const hourlyEarning = 0.117 + deterministicNoise(i + 200, 0.03);
    cumulative += hourlyEarning;
    points.push({
      timestamp: ts,
      time: formatDate(ts),
      pnl: Number(cumulative.toFixed(2)),
      fundingEarned: Number(cumulative.toFixed(2)),
    });
  }
  return points;
}

// ── Helpers (deterministic, no Math.random) ──

function seedTimestamp(offsetHours: number): number {
  // Fixed reference: 2026-04-09 12:00:00 UTC
  return 1775908800 + offsetHours * 3600;
}

/** Deterministic noise using a simple hash — no randomness */
function deterministicNoise(seed: number, amplitude: number): number {
  const x = Math.sin(seed * 12.9898 + 78.233) * 43758.5453;
  return (x - Math.floor(x) - 0.5) * 2 * amplitude;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
}

function formatDate(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
}
