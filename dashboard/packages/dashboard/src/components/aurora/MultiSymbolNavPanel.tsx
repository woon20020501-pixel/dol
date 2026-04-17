"use client";

import { useMemo } from "react";
import {
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  AURORA_CONSTANTS,
  SYMBOL_UNIVERSE,
  useAuroraTelemetry,
  type AuroraTelemetry,
  type SymbolState,
} from "@/hooks/useAuroraTelemetry";
import { AnimatedNumber } from "@/components/common/AnimatedNumber";

/**
 * Multi-symbol NAV panel — the high-dimensional viz called for in the spec
 * . Layers:
 *
 *   1. Hierarchical line chart
 *      - 1 THICK aggregate line (10-pair portfolio NAV vs $10k seed)
 *      - top-3 per-pair lines highlighted in their brand color
 *      - remaining 7 pairs muted in dark-tertiary
 *      - top-3 set re-ranks live as BTC's outlier lead fades and BNB/SOL
 *        catch up
 *   2. Sparkline strip — 10 tiles showing each symbol's NAV trajectory
 *      and live gain, sorted by current rank
 *   3. "10 → 46 pairs" production-bridge caption with canonical talking
 *      points from 
 */
export function MultiSymbolNavPanel() {
  const t = useAuroraTelemetry();
  return (
    <section
      aria-label="Multi-symbol NAV trajectory"
      className="relative mt-6 overflow-hidden rounded-2xl border border-senior/20 bg-gradient-to-br from-dark-surface via-dark-surface to-[#0b1315]"
    >
      <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(to_right,rgba(45,212,191,0.04)_1px,transparent_1px),linear-gradient(to_bottom,rgba(45,212,191,0.04)_1px,transparent_1px)] bg-[size:56px_56px]" />
      <div className="pointer-events-none absolute -right-24 top-0 h-[300px] w-[300px] rounded-full bg-senior/10 blur-[140px]" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-gradient-to-r from-transparent via-senior/50 to-transparent" />

      <div className="relative z-10 p-4 sm:p-6">
        <PanelHeader telemetry={t} />
        <MultiLineChart telemetry={t} />
        <SparklineStrip symbols={t.symbols} />
        <BridgeCaption />
      </div>
    </section>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Header
// ──────────────────────────────────────────────────────────────────────────

function PanelHeader({ telemetry }: { telemetry: AuroraTelemetry }) {
  const { aggregate, simElapsedHours } = telemetry;
  const pnlPositive = aggregate.pnlUsd >= 0;
  const pnlTone = pnlPositive ? "text-senior" : "text-carry-red";

  return (
    <div className="flex flex-wrap items-start justify-between gap-6">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inset-0 animate-ping rounded-full bg-senior/50" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-senior" />
          </span>
          <p className="font-mono text-[10px] uppercase tracking-[0.18em] text-senior/80">
            Portfolio · 10-symbol universe · 7 crypto + 3 RWA
          </p>
        </div>
        <p className="mt-1.5 text-[18px] font-semibold tracking-[-0.01em] text-dark-primary">
          Aggregate NAV trajectory
        </p>
        <p className="mt-0.5 font-mono text-[11px] text-dark-tertiary">
          accel {AURORA_CONSTANTS.accelFactor}× · sim T+{simElapsedHours.toFixed(1)}h ·{" "}
          {aggregate.activePairs} pairs ·{" "}
          {aggregate.breakevenReached ? (
            <span className="text-senior">breakeven ✓</span>
          ) : (
            <span className="text-carry-amber">
              T−{aggregate.hoursToBreakeven.toFixed(1)}h
            </span>
          )}
        </p>
      </div>

      <div className="shrink-0 text-right font-mono">
        <div className="text-[10px] uppercase tracking-[0.08em] text-dark-tertiary">
          Aggregate NAV
        </div>
        <div className="mt-0.5 text-[30px] font-semibold leading-none tracking-[-0.02em] text-dark-primary tabular-nums">
          $
          <AnimatedNumber
            value={aggregate.navUsd}
            format={(v) =>
              v.toLocaleString("en-US", {
                minimumFractionDigits: 2,
                maximumFractionDigits: 2,
              })
            }
            duration={160}
          />
        </div>
        <div className={`mt-1 text-[12px] tabular-nums ${pnlTone}`}>
          {pnlPositive ? "+" : ""}
          <AnimatedNumber value={aggregate.pnlUsd} format={(v) => v.toFixed(2)} duration={160} />
          {" USD · "}
          {pnlPositive ? "+" : ""}
          <AnimatedNumber value={aggregate.pnlBps} format={(v) => v.toFixed(2)} duration={160} />
          {" bps"}
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Multi-line chart
// ──────────────────────────────────────────────────────────────────────────

type ChartRow = { t: number; agg: number } & Record<string, number>;

function MultiLineChart({ telemetry }: { telemetry: AuroraTelemetry }) {
  const { symbols, aggregate, simElapsedHours } = telemetry;

  // Build top-3 set — recomputed per render as rankings may change.
  const top3 = useMemo(() => {
    return symbols
      .slice()
      .sort((a, b) => b.pnlPair - a.pnlPair)
      .slice(0, 3)
      .map((s) => s.symbol);
  }, [symbols]);

  // Merge per-symbol + aggregate series into a single dataset indexed by t.
  // All series share the same t grid after the chartWindowHours fix, so the
  // zip is a simple index walk. Per-pair navs are interpolated against the
  // aggregate's canonical x-grid for safety.
  const data: ChartRow[] = useMemo(() => {
    const anchorSeries = aggregate.series;
    return anchorSeries.map((pt) => {
      const row: ChartRow = { t: pt.t, agg: pt.nav };
      for (const s of symbols) {
        row[s.symbol] = interpolateNav(s.series, pt.t);
      }
      return row;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [aggregate.series, symbols]);

  // Scale normalization (this is the big fix):
  //   - Per-pair bps  = (nav − $100) / $100 × 10000
  //   - Aggregate bps = (nav − $10k) / $1k × 10000
  //     NOTE: denominator is DEPLOYED capital ($1,000), not the $10k seed.
  //     This puts the aggregate curve in the same bps range as the mean of
  //     the per-pair curves — no more flat teal line pinned to the 0 axis
  //     while pairs climb to 800+.
  const bpsData = useMemo(() => {
    return data.map((row) => {
      const out: ChartRow = {
        t: row.t,
        agg:
          ((row.agg - AURORA_CONSTANTS.startingNavUsd) /
            AURORA_CONSTANTS.deployedUsdDemo) *
          10_000,
      };
      for (const s of symbols) {
        out[s.symbol] =
          ((row[s.symbol] - AURORA_CONSTANTS.pairNotionalUsd) /
            AURORA_CONSTANTS.pairNotionalUsd) *
          10_000;
      }
      return out;
    });
  }, [data, symbols]);

  const windowHours = AURORA_CONSTANTS.chartWindowHours;
  const tMarker = Math.min(simElapsedHours, windowHours);

  // Show the aggregate + top-3 in the tooltip; muted tail pairs would
  // just be 7 lines of noise at 11 pts font.
  const visibleInTooltip = new Set<string>(["AGGREGATE", ...top3]);

  return (
    <div className="mt-4">
      <div className="h-[280px] w-full">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart
            data={bpsData}
            margin={{ top: 14, right: 16, bottom: 14, left: 8 }}
          >
            <defs>
              <linearGradient id="aurora-agg-fill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="#2dd4bf" stopOpacity={0.18} />
                <stop offset="100%" stopColor="#2dd4bf" stopOpacity={0} />
              </linearGradient>
            </defs>
            <XAxis
              dataKey="t"
              type="number"
              domain={[0, windowHours]}
              ticks={[0, 24, 48, 72, 96, 120]}
              tickFormatter={(v) => `${Number(v).toFixed(0)}h`}
              stroke="#5a5a5f"
              tick={{ fill: "#86868b", fontSize: 10, fontFamily: "monospace" }}
              tickLine={false}
              axisLine={{ stroke: "#2a2a2d" }}
              allowDataOverflow
            />
            <YAxis
              stroke="#5a5a5f"
              tick={{ fill: "#86868b", fontSize: 10, fontFamily: "monospace" }}
              tickLine={false}
              axisLine={{ stroke: "#2a2a2d" }}
              tickFormatter={(v) => `${Number(v).toFixed(0)} bp`}
              width={52}
              domain={["auto", "auto"]}
            />
            <Tooltip
              cursor={{ stroke: "#2dd4bf", strokeDasharray: "3 3" }}
              contentStyle={{
                background: "#0a0a0b",
                border: "1px solid #2dd4bf",
                borderRadius: 8,
                fontFamily: "monospace",
                fontSize: 10,
              }}
              labelFormatter={(v) => `sim +${Number(v).toFixed(1)}h`}
              formatter={(v, name) => {
                if (!visibleInTooltip.has(String(name))) return [null, null];
                return [`${Number(v).toFixed(2)} bp`, name];
              }}
              itemSorter={(item) => -Number(item.value ?? 0)}
            />

            {/* Break line (0 bp) — label on the LEFT inside so it doesn't
                collide with the top-3 curves at the right edge. */}
            <ReferenceLine
              y={0}
              stroke="#5a5a5f"
              strokeDasharray="2 4"
              label={{
                value: "break (0 bp)",
                fill: "#86868b",
                fontSize: 9,
                position: "insideBottomLeft",
              }}
            />

            {/* Aggregate breakeven vertical */}
            <ReferenceLine
              x={aggregate.breakevenHours}
              stroke="#ff9f0a"
              strokeDasharray="2 2"
              label={{
                value: `agg breakeven · ${aggregate.breakevenHours.toFixed(0)}h`,
                fill: "#ff9f0a",
                fontSize: 9,
                position: "insideTopRight",
              }}
            />

            {/* Live sim-time marker — the chart's "heartbeat". Clamped at
                the window edge once sim time exceeds chartWindowHours. */}
            <ReferenceLine
              x={tMarker}
              stroke="#2dd4bf"
              strokeWidth={1.5}
              strokeOpacity={0.9}
            />

            {/* 7 muted tail pairs first (painted under) */}
            {SYMBOL_UNIVERSE.filter((s) => !top3.includes(s.symbol)).map((s) => (
              <Line
                key={`tail-${s.symbol}`}
                type="monotone"
                dataKey={s.symbol}
                name={s.symbol}
                stroke="#3a3a3e"
                strokeWidth={1}
                dot={false}
                isAnimationActive={false}
                strokeOpacity={0.5}
              />
            ))}

            {/* top-3 highlighted in brand color */}
            {SYMBOL_UNIVERSE.filter((s) => top3.includes(s.symbol)).map((s) => (
              <Line
                key={`top-${s.symbol}`}
                type="monotone"
                dataKey={s.symbol}
                name={s.symbol}
                stroke={s.color}
                strokeWidth={1.75}
                dot={false}
                isAnimationActive={false}
                strokeOpacity={0.95}
              />
            ))}

            {/* Thick aggregate on top — normalized to deployed capital now */}
            <Line
              type="monotone"
              dataKey="agg"
              name="AGGREGATE"
              stroke="#2dd4bf"
              strokeWidth={3}
              dot={false}
              isAnimationActive={false}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>

      {/* Legend */}
      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-[10px]">
        <LegendDot color="#2dd4bf" thick label="AGGREGATE" />
        {top3.map((sym) => {
          const spec = SYMBOL_UNIVERSE.find((s) => s.symbol === sym);
          if (!spec) return null;
          return <LegendDot key={sym} color={spec.color} label={sym} />;
        })}
        <span className="text-dark-tertiary">+ {10 - top3.length} muted</span>
        <span className="ml-auto text-dark-tertiary">
          x-window: fixed 0–{windowHours}h · y-scale: bps on deployed
        </span>
      </div>
    </div>
  );
}

function LegendDot({
  color,
  label,
  thick = false,
}: {
  color: string;
  label: string;
  thick?: boolean;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 text-dark-secondary">
      <span
        className="inline-block rounded-full"
        style={{
          background: color,
          width: thick ? 10 : 8,
          height: thick ? 3 : 2,
        }}
      />
      {label}
    </span>
  );
}

function interpolateNav(series: { t: number; nav: number }[], t: number): number {
  if (series.length === 0) return 0;
  if (t <= series[0].t) return series[0].nav;
  if (t >= series[series.length - 1].t) return series[series.length - 1].nav;
  for (let i = 1; i < series.length; i++) {
    if (series[i].t >= t) {
      const a = series[i - 1];
      const b = series[i];
      const k = (t - a.t) / (b.t - a.t);
      return a.nav + (b.nav - a.nav) * k;
    }
  }
  return series[series.length - 1].nav;
}

// ──────────────────────────────────────────────────────────────────────────
// Sparkline strip
// ──────────────────────────────────────────────────────────────────────────

function SparklineStrip({ symbols }: { symbols: SymbolState[] }) {
  const sorted = useMemo(
    () => symbols.slice().sort((a, b) => a.rank - b.rank),
    [symbols],
  );
  return (
    <div className="mt-5 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
      {sorted.map((s) => (
        <SparklineTile key={s.symbol} s={s} />
      ))}
    </div>
  );
}

function SparklineTile({ s }: { s: SymbolState }) {
  const gain = s.pnlPair;
  const positive = gain >= 0;
  const gainTone = positive ? "text-senior" : "text-carry-red";
  const tone = positive
    ? "border-senior/25 bg-senior/[0.04]"
    : "border-dark-border bg-dark-surface-2/60";
  const path = useMemo(() => buildSparklinePath(s.series), [s.series]);

  return (
    <div
      className={`group relative overflow-hidden rounded-lg border ${tone} p-2.5 transition-colors`}
    >
      {/* Row 1: rank, symbol, kind + divergence icon */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="font-mono text-[9px] text-dark-tertiary tabular-nums w-4">
            #{s.rank}
          </span>
          <span
            className="h-1.5 w-1.5 rounded-full shrink-0"
            style={{ background: s.color }}
            aria-hidden="true"
          />
          <span className="font-mono text-[12px] font-semibold text-dark-primary truncate">
            {s.symbol}
          </span>
        </div>
        <div className="flex items-center gap-1">
          {s.kind === "rwa" ? (
            <span
              className="rounded border border-junior/40 bg-junior/10 px-1 py-[0.5px] font-mono text-[8px] font-semibold uppercase text-junior"
              title="Real-world asset pair"
            >
              RWA
            </span>
          ) : null}
          {s.bookParseDegraded ? (
            <span
              className="font-mono text-[10px] leading-none text-dark-tertiary"
              title={
                s.kind === "rwa"
                  ? `Reduced venue coverage: Pacifica + Hyperliquid only. Lighter and Backpack don't list ${s.symbol} (gold/silver commodities). The bot routes ${s.symbol} through the 2-venue pair Expected operational state, not a fault.`
                  : `Fetch failures detected — investigate adapter logs for ${s.symbol}.`
              }
              aria-label={
                s.kind === "rwa"
                  ? "Reduced venue coverage (expected for RWA)"
                  : "Book parse degraded — investigate"
              }
            >
              ⚪
            </span>
          ) : null}
          {s.oracleDivergenceRisk === "structural" ? (
            <span
              className="font-mono text-[10px] leading-none text-carry-amber"
              title="RWA hedge: independent oracles on each leg. Documented tail risk, sized by venue concentration caps."
              aria-label="Oracle divergence risk: structural"
            >
              ⚠
            </span>
          ) : null}
        </div>
      </div>

      {/* Sparkline */}
      <svg
        className="mt-2 block w-full"
        viewBox="0 0 100 24"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <path
          d={path}
          fill="none"
          stroke={s.color}
          strokeWidth={1.5}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>

      {/* Row 3: gain + spread, tighter */}
      <div className="mt-1.5 flex items-baseline justify-between gap-2 font-mono tabular-nums">
        <span className={`text-[12px] ${gainTone}`}>
          {positive ? "+" : ""}${gain.toFixed(2)}
        </span>
        <span className="text-[9px] text-dark-tertiary">
          {s.spreadPct.toFixed(1)}%
        </span>
      </div>
    </div>
  );
}

function buildSparklinePath(series: { t: number; nav: number }[]): string {
  if (series.length === 0) return "";
  const navs = series.map((p) => p.nav);
  const min = Math.min(...navs);
  const max = Math.max(...navs);
  const range = max - min || 1;
  return series
    .map((p, i) => {
      const x = (i / (series.length - 1)) * 100;
      const y = 22 - ((p.nav - min) / range) * 22;
      return `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
}

// ──────────────────────────────────────────────────────────────────────────
// Bridge caption
// ──────────────────────────────────────────────────────────────────────────

function BridgeCaption() {
  return (
    <div className="mt-5 rounded-xl border border-dark-border bg-black/30 p-4">
      <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
        demo → production bridge
      </p>
      <div className="mt-2 grid gap-2 font-mono text-[11px] sm:grid-cols-3">
        <BridgeCell
          label="universe"
          demo="10 · 7 crypto + 3 RWA"
          prod="46 pairs"
        />
        <BridgeCell
          label="deployed"
          demo="$1,000 · 10% NAV"
          prod="50% AUM"
        />
        <BridgeCell
          label="customer APY (capped)"
          demo="5–7.5% band"
          prod="8.00% · 14.2% gross"
        />
      </div>
      <p className="mt-3 font-mono text-[10px] leading-relaxed text-dark-tertiary">
        Dol&apos;s real yield mandate: funding-spread capture over a universe of
        crypto pairs (BTC anomaly, AVAX strongest crypto spread) plus Pacifica
        ↔ trade.xyz RWA pairs (XAU, XAG, PAXG — PAXG is the most persistent
        opportunity at 312h same-direction, 2 sign flips in 21 days). RWA
        rows carry an{" "}
        <span className="text-carry-amber">⚠ oracle-divergence</span> flag:
        independent oracles on each leg, documented structural risk sized by
        venue concentration caps. Production scales to 46 pairs at 50%
        deployment — mandate ceiling 8.00% customer / 4.78% buffer / 1.42%
        reserve under research's 60-day backtest.
      </p>
    </div>
  );
}

function BridgeCell({
  label,
  demo,
  prod,
}: {
  label: string;
  demo: string;
  prod: string;
}) {
  return (
    <div className="rounded-lg border border-dark-border/70 bg-dark-surface-2/50 p-2.5">
      <div className="text-[9px] uppercase tracking-[0.08em] text-dark-tertiary">
        {label}
      </div>
      <div className="mt-1 flex items-center gap-1.5">
        <span className="text-dark-primary">{demo}</span>
        <span className="text-dark-tertiary">→</span>
        <span className="text-senior">{prod}</span>
      </div>
    </div>
  );
}
