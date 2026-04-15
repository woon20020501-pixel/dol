"use client";

import { motion, AnimatePresence } from "framer-motion";
import {
  Area,
  AreaChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  AURORA_CONSTANTS,
  useAuroraTelemetry,
  type AuroraTelemetry,
  type DecisionEvent,
  type RiskLayer,
} from "@/hooks/useAuroraTelemetry";
import { AnimatedNumber } from "@/components/common/AnimatedNumber";

/**
 * Aurora-Ω Operate Console — the "alive" Week-1 demo layer on top of the
 * existing operator dashboard. Visualizes the Rust bot telemetry:
 *
 *   - NAV pulse curve (entry cost step-down → slow accrual climb to breakeven)
 *   - Venue health grid (Pacifica live, other 3 Week-1 fixtures)
 *   - Funding cycle lock ring (1-hour Pacifica cycle, I-LOCK enforced)
 *   - Pair decision card (long/short venue, spread, notional)
 *   - Risk-stack monitor (4 layers, stub-flagged for Week-1 scope)
 *   - Decision log ticker (scrolling JARVIS-style)
 *   - v0 mode badge (which framework layers are intentionally stubbed)
 *
 * Data source: client-side deterministic simulator seeded from the
 * authoritative runtime numbers. When the Rust bot's `output/nav.jsonl`
 * stream is live, swap the hook's inner source — UI stays the same.
 */
export function AuroraConsole() {
  const t = useAuroraTelemetry();

  return (
    <section
      aria-label="Aurora-Omega Operate Console"
      className="relative mb-6 overflow-hidden rounded-2xl border border-senior/25 bg-gradient-to-br from-dark-surface via-dark-surface to-[#0b1816]"
    >
      {/* Animated grid backdrop */}
      <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(to_right,rgba(45,212,191,0.045)_1px,transparent_1px),linear-gradient(to_bottom,rgba(45,212,191,0.045)_1px,transparent_1px)] bg-[size:40px_40px]" />
      <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-senior/60 to-transparent" />
      <div className="pointer-events-none absolute -left-24 top-1/2 h-[220px] w-[220px] -translate-y-1/2 rounded-full bg-senior/10 blur-[120px]" />

      <div className="relative z-10 p-4 sm:p-6">
        <ConsoleHeader telemetry={t} />

        <div className="mt-4 space-y-4">
          <NavPulsePanel telemetry={t} />
          <div className="grid gap-4 md:grid-cols-[1fr_1fr_320px]">
            <PairDecisionCard telemetry={t} />
            <CycleLockRing telemetry={t} />
            <RiskStackMonitor stack={t.riskStack} />
          </div>
        </div>

        <DecisionTicker decisions={t.decisions} />

        <ConsoleFootnote />
      </div>
    </section>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Header
// ──────────────────────────────────────────────────────────────────────────

function ConsoleHeader({ telemetry }: { telemetry: AuroraTelemetry }) {
  const simHoursDisplay = telemetry.simElapsedHours.toFixed(1);
  const { dataSource } = telemetry;

  // Honest status line replaces the old hardcoded "LIVE" label: it now
  // reflects whether the hook is actually consuming nav.jsonl rows or
  // the deterministic simulator fallback.
  const statusLabel =
    dataSource === "LIVE"
      ? "Funding-capture engine · live nav.jsonl"
      : dataSource === "STALE"
        ? "Funding-capture engine · nav.jsonl stale"
        : "Funding-capture engine · simulator fallback";

  const statusAccent =
    dataSource === "LIVE"
      ? "text-senior"
      : dataSource === "STALE"
        ? "text-carry-amber"
        : "text-dark-tertiary";

  const dotBg =
    dataSource === "LIVE"
      ? "bg-senior"
      : dataSource === "STALE"
        ? "bg-carry-amber"
        : "bg-dark-tertiary";

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-3">
        <div className="relative flex h-8 w-8 items-center justify-center">
          {dataSource === "LIVE" ? (
            <span className="absolute inset-0 animate-ping rounded-full bg-senior/30" />
          ) : null}
          <span className={`absolute inset-[6px] rounded-full ${dotBg}`} />
        </div>
        <div>
          <p className="font-mono text-[10px] uppercase tracking-[0.18em] text-senior/80">
            AURORA-Ω · WEEK 1 DEMO CONSOLE
          </p>
          <p className={`text-[18px] font-semibold tracking-[-0.01em] text-dark-primary`}>
            {statusLabel.split("·")[0]}·{" "}
            <span className={statusAccent}>
              {statusLabel.split("·").slice(1).join("·").trim()}
            </span>
          </p>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 font-mono text-[10px] uppercase tracking-[0.08em]">
        <DataSourceBadge source={dataSource} />
        <Pill tone="senior">ACCEL {AURORA_CONSTANTS.accelFactor}×</Pill>
        <Pill tone="senior">T+{simHoursDisplay}h</Pill>
        <Pill tone={telemetry.fsmMode === "nominal" ? "senior" : "amber"}>
          FSM · {telemetry.fsmMode.toUpperCase()}
        </Pill>
        <Pill tone="gray">V0 · {telemetry.stubbedSections.length} STUBBED</Pill>
      </div>
    </div>
  );
}

function DataSourceBadge({ source }: { source: AuroraTelemetry["dataSource"] }) {
  if (source === "LIVE") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full border border-senior/50 bg-senior/15 px-2 py-0.5 text-senior">
        <span className="relative flex h-1.5 w-1.5">
          <span className="absolute inset-0 animate-ping rounded-full bg-senior/60" />
          <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-senior" />
        </span>
        LIVE
      </span>
    );
  }
  if (source === "STALE") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full border border-carry-amber/50 bg-carry-amber/15 px-2 py-0.5 text-carry-amber">
        <span className="h-1.5 w-1.5 rounded-full bg-carry-amber" />
        STALE
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 rounded-full border border-dark-border bg-dark-surface-2 px-2 py-0.5 text-dark-secondary">
      <span className="h-1.5 w-1.5 rounded-full bg-dark-tertiary" />
      SIM
    </span>
  );
}

function Pill({
  children,
  tone,
}: {
  children: React.ReactNode;
  tone: "senior" | "amber" | "gray" | "red";
}) {
  const toneClass = {
    senior: "border-senior/40 bg-senior/10 text-senior",
    amber: "border-carry-amber/40 bg-carry-amber/10 text-carry-amber",
    gray: "border-dark-border bg-dark-surface-2 text-dark-secondary",
    red: "border-carry-red/40 bg-carry-red/10 text-carry-red",
  }[tone];
  return (
    <span className={`rounded-full border px-2 py-0.5 ${toneClass}`}>
      {children}
    </span>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// NAV pulse chart (P0)
// ──────────────────────────────────────────────────────────────────────────

function NavPulsePanel({ telemetry }: { telemetry: AuroraTelemetry }) {
  const { navUsd, breakevenReached, hoursToBreakeven, navSeries } = telemetry;
  const pnlUsd = navUsd - AURORA_CONSTANTS.startingNavUsd;
  const pnlBps = (pnlUsd / AURORA_CONSTANTS.startingNavUsd) * 10_000;
  const pnlTone = pnlUsd >= 0 ? "text-senior" : "text-carry-red";

  // Y-axis: fit the actual series + current nav. Do NOT force-include
  // the $10k seed line — if live nav is at $10,088 and seed is $10,000,
  // including seed squashes the real movement into the top 1% of the
  // chart. Seed reference is rendered only when it lies inside the
  // computed range.
  const seed = AURORA_CONSTANTS.startingNavUsd;
  const seriesValues = navSeries.map((p) => p.nav);
  const allValues =
    seriesValues.length > 0 ? [...seriesValues, navUsd] : [navUsd];
  const dataMin = Math.min(...allValues);
  const dataMax = Math.max(...allValues);
  // Minimum visible range: $0.50 floor so tiny-variance live windows
  // still show meaningful vertical travel instead of a flat line.
  const baseRange = Math.max(dataMax - dataMin, 0.5);
  const pad = baseRange * 0.2;
  const yMin = dataMin - pad;
  const yMax = dataMax + pad;
  const seedInRange = seed >= yMin && seed <= yMax;

  // X-axis: compute data t-range explicitly so Recharts doesn't stretch
  // the axis to include an off-frame ReferenceLine (which squashed the
  // live curve into the left 5% when breakeven=85h but live t-span=4h).
  const tValues = navSeries.map((p) => p.t);
  const tMin = tValues.length ? Math.min(...tValues) : 0;
  const tMax = tValues.length ? Math.max(...tValues) : 120;
  const breakeven = AURORA_CONSTANTS.breakevenHours;
  const breakevenInRange = breakeven >= tMin && breakeven <= tMax;

  return (
    <div className="rounded-xl border border-dark-border bg-dark-surface/60 p-4 backdrop-blur-sm">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
            NAV pulse · $10k seed · net of $0.10 entry
          </p>
          <p className="mt-1 font-mono text-[32px] font-semibold leading-none tracking-[-0.02em] text-dark-primary">
            $<AnimatedNumber
              value={navUsd}
              format={(v) => v.toFixed(4)}
              duration={200}
            />
          </p>
          <p className={`mt-1 font-mono text-[12px] ${pnlTone}`}>
            {pnlUsd >= 0 ? "+" : ""}
            {pnlUsd.toFixed(4)} USD · {pnlBps >= 0 ? "+" : ""}
            {pnlBps.toFixed(3)} bps
          </p>
        </div>
        <div className="text-right font-mono text-[11px] text-dark-secondary">
          <div>
            breakeven:{" "}
            <span className="text-dark-primary">
              {AURORA_CONSTANTS.breakevenHours.toFixed(1)}h
            </span>
          </div>
          <div>
            {breakevenReached ? (
              <span className="text-senior">✓ crossed</span>
            ) : (
              <span className="text-carry-amber">
                T-{hoursToBreakeven.toFixed(1)}h
              </span>
            )}
          </div>
          <div>
            1y projection:{" "}
            <span className="text-senior">+$18.82</span>
          </div>
        </div>
      </div>

      <div className="mt-3 h-[150px] w-full">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart
            data={navSeries}
            margin={{ top: 4, right: 8, bottom: 4, left: 8 }}
          >
            <defs>
              <linearGradient id="aurora-nav" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="#2dd4bf" stopOpacity={0.5} />
                <stop offset="100%" stopColor="#2dd4bf" stopOpacity={0} />
              </linearGradient>
            </defs>
            <XAxis
              dataKey="t"
              type="number"
              domain={[tMin, tMax]}
              allowDataOverflow
              tickFormatter={(v) => `${Number(v).toFixed(1)}h`}
              stroke="#5a5a5f"
              tick={{ fill: "#86868b", fontSize: 10, fontFamily: "monospace" }}
              tickLine={false}
              axisLine={{ stroke: "#2a2a2d" }}
            />
            <YAxis
              domain={[yMin, yMax]}
              tickFormatter={(v) => Number(v).toFixed(2)}
              stroke="#5a5a5f"
              tick={{ fill: "#86868b", fontSize: 10, fontFamily: "monospace" }}
              tickLine={false}
              axisLine={{ stroke: "#2a2a2d" }}
              width={60}
            />
            <Tooltip
              cursor={{ stroke: "#2dd4bf", strokeDasharray: "3 3" }}
              contentStyle={{
                background: "#0a0a0b",
                border: "1px solid #2dd4bf",
                borderRadius: 8,
                fontFamily: "monospace",
                fontSize: 11,
              }}
              labelFormatter={(v) => `sim +${Number(v).toFixed(2)}h`}
              formatter={(v) => [`$${Number(v).toFixed(4)}`, "NAV"]}
            />
            {seedInRange ? (
              <ReferenceLine
                y={AURORA_CONSTANTS.startingNavUsd}
                stroke="#86868b"
                strokeDasharray="2 4"
                label={{
                  value: "seed $10,000",
                  fill: "#86868b",
                  fontSize: 9,
                  position: "insideTopLeft",
                }}
              />
            ) : null}
            {breakevenInRange ? (
              <ReferenceLine
                x={AURORA_CONSTANTS.breakevenHours}
                stroke="#ff9f0a"
                strokeDasharray="2 2"
                label={{
                  value: "breakeven",
                  fill: "#ff9f0a",
                  fontSize: 9,
                  position: "top",
                }}
              />
            ) : null}
            <Area
              type="monotone"
              dataKey="nav"
              stroke="#2dd4bf"
              strokeWidth={2}
              fill="url(#aurora-nav)"
              isAnimationActive={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>

      <div className="mt-2 flex items-center justify-between font-mono text-[10px] text-dark-tertiary">
        <span>
          source:{" "}
          {telemetry.dataSource === "LIVE"
            ? "nav.jsonl · live poll 2s"
            : telemetry.dataSource === "STALE"
              ? "nav.jsonl · stale > 30s (last known)"
              : "deterministic simulator · fallback"}
          {" · accel "}{AURORA_CONSTANTS.accelFactor}×
        </span>
        <span>event: {telemetry.positionEvent}</span>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Pair decision card
// ──────────────────────────────────────────────────────────────────────────

function PairDecisionCard({ telemetry }: { telemetry: AuroraTelemetry }) {
  const { pairDecision } = telemetry;
  const spreadHeld = /no-rebalance|hold/i.test(pairDecision.reason);
  return (
    <div className="rounded-xl border border-dark-border bg-dark-surface/60 p-4">
      <div className="flex items-center justify-between">
        <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
          extra.pair_decision
        </p>
        <SignalAgeBadge telemetry={telemetry} />
      </div>
      <div className="mt-3 flex items-center gap-2 font-mono text-[13px]">
        <span className="rounded border border-senior/40 bg-senior/10 px-1.5 py-0.5 text-senior">
          LONG
        </span>
        <span className="text-dark-primary">{pairDecision.longVenue}</span>
        <span className="text-dark-tertiary">/</span>
        <span className="rounded border border-carry-red/40 bg-carry-red/10 px-1.5 py-0.5 text-carry-red">
          SHORT
        </span>
        <span className="text-dark-primary">{pairDecision.shortVenue}</span>
      </div>
      <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1.5 font-mono text-[11px]">
        <dt className="text-dark-tertiary">symbol</dt>
        <dd className="text-right text-dark-primary">{AURORA_CONSTANTS.symbol}</dd>
        <dt className="text-dark-tertiary">
          spread (pa)
          {spreadHeld ? (
            <span
              className="ml-1 cursor-help text-carry-amber"
              title="Pair opened with this spread snapshot. Bot's demo no-rebalance policy holds the value for the full cycle — it doesn't recompute until a new cycle opens."
            >
              🔒
            </span>
          ) : null}
        </dt>
        <dd className="text-right text-senior">
          {pairDecision.spreadAnnualPct.toFixed(2)}% ·{" "}
          {Math.round(pairDecision.spreadAnnualPct * 100)} bps
        </dd>
        <dt className="text-dark-tertiary">notional</dt>
        <dd className="text-right text-dark-primary">
          ${pairDecision.notionalUsd.toFixed(2)} <span className="text-dark-tertiary">/ 1% NAV</span>
        </dd>
        <dt className="text-dark-tertiary">reason</dt>
        <dd className="text-right text-dark-primary truncate" title={pairDecision.reason}>
          {pairDecision.reason.length > 24
            ? pairDecision.reason.slice(0, 24) + "…"
            : pairDecision.reason}
        </dd>
        <dt className="text-dark-tertiary">positions</dt>
        <dd className="text-right text-dark-primary">
          1 <span className="text-dark-tertiary">/ 46 configured</span>
        </dd>
      </dl>
      <p className="mt-3 font-mono text-[10px] text-dark-tertiary">
        dry-run · would_have_executed=true · RUNNER_ALLOW_LIVE=false
      </p>
    </div>
  );
}

function SignalAgeBadge({ telemetry }: { telemetry: AuroraTelemetry }) {
  const receivedAt = telemetry.liveSignalReceivedAtMs;
  const source = telemetry.liveSignalSource;
  if (receivedAt === null || source === "SIM") {
    return (
      <span className="rounded-full border border-dark-border bg-dark-surface-2 px-1.5 py-0.5 font-mono text-[9px] text-dark-tertiary">
        sim
      </span>
    );
  }
  // Rough age — rAF ticker upstream drives re-renders so this updates live.
  const ageSec = Math.max(0, (Date.now() - receivedAt) / 1000);
  const tone =
    ageSec < 10
      ? "border-senior/40 bg-senior/10 text-senior"
      : ageSec < 30
        ? "border-carry-amber/40 bg-carry-amber/10 text-carry-amber"
        : "border-carry-red/40 bg-carry-red/10 text-carry-red";
  return (
    <span className={`inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 font-mono text-[9px] ${tone}`}>
      <span
        className={`h-1 w-1 rounded-full ${ageSec < 10 ? "bg-senior" : ageSec < 30 ? "bg-carry-amber" : "bg-carry-red"}`}
      />
      signal · {ageSec.toFixed(0)}s ago
    </span>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Cycle lock ring
// ──────────────────────────────────────────────────────────────────────────

function CycleLockRing({ telemetry }: { telemetry: AuroraTelemetry }) {
  const { cycleLock } = telemetry;
  const size = 110;
  const stroke = 8;
  const r = (size - stroke) / 2;
  const circ = 2 * Math.PI * r;
  const dash = circ * (1 - cycleLock.cycleProgress);
  const mins = Math.floor(cycleLock.secondsToCycleEnd / 60);
  const secs = Math.floor(cycleLock.secondsToCycleEnd % 60);

  return (
    <div className="rounded-xl border border-dark-border bg-dark-surface/60 p-4">
      <div className="flex items-start justify-between">
        <div>
          <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
            funding cycle lock
          </p>
          <p className="mt-1 font-mono text-[11px] text-senior">
            {cycleLock.locked ? "I-LOCK · ENGAGED" : "unlocked"}
          </p>
        </div>
        <Pill tone={cycleLock.proposedWasBlocked ? "amber" : "senior"}>
          {cycleLock.proposedWasBlocked ? "FLIP BLOCKED" : "HELD"}
        </Pill>
      </div>
      <div className="mt-2 flex items-center gap-4">
        <div className="relative" style={{ width: size, height: size }}>
          <svg width={size} height={size} className="-rotate-90">
            <circle
              cx={size / 2}
              cy={size / 2}
              r={r}
              stroke="#2a2a2d"
              strokeWidth={stroke}
              fill="none"
            />
            <circle
              cx={size / 2}
              cy={size / 2}
              r={r}
              stroke="#2dd4bf"
              strokeWidth={stroke}
              fill="none"
              strokeDasharray={circ}
              strokeDashoffset={dash}
              strokeLinecap="round"
              style={{ transition: "stroke-dashoffset 0.1s linear" }}
            />
          </svg>
          <div className="absolute inset-0 flex flex-col items-center justify-center font-mono">
            <span className="text-[16px] font-semibold text-dark-primary">
              {String(mins).padStart(2, "0")}:{String(secs).padStart(2, "0")}
            </span>
            <span className="text-[9px] uppercase tracking-[0.1em] text-dark-tertiary">
              to cycle end
            </span>
          </div>
        </div>
        <dl className="flex-1 space-y-1 font-mono text-[11px]">
          <div className="flex justify-between">
            <dt className="text-dark-tertiary">cycle c</dt>
            <dd className="text-dark-primary">{cycleLock.cycleIndex}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-dark-tertiary">h_c</dt>
            <dd className="text-dark-primary">{cycleLock.hC}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-dark-tertiary">N_c</dt>
            <dd className="text-dark-primary">${cycleLock.nC.toFixed(0)}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-dark-tertiary">window</dt>
            <dd className="text-dark-primary">3600s</dd>
          </div>
        </dl>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Risk stack monitor
// ──────────────────────────────────────────────────────────────────────────

function RiskStackMonitor({ stack }: { stack: RiskLayer[] }) {
  return (
    <div className="rounded-xl border border-dark-border bg-dark-surface/60 p-4">
      <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
        risk stack · 4 layers · 2-of-4 fail-safe
      </p>
      <ul className="mt-3 space-y-2">
        {stack.map((layer) => (
          <li
            key={layer.name}
            className="flex items-center justify-between rounded-lg border border-dark-border/70 bg-dark-surface-2/60 px-2.5 py-1.5"
          >
            <div className="flex items-center gap-2">
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  layer.red
                    ? "bg-carry-red"
                    : layer.stub
                      ? "bg-dark-tertiary"
                      : "bg-senior"
                }`}
              />
              <span
                className={`font-mono text-[11px] ${
                  layer.stub ? "text-dark-tertiary" : "text-dark-primary"
                }`}
              >
                {layer.label}
              </span>
            </div>
            <span className="font-mono text-[9px] uppercase tracking-[0.08em]">
              {layer.stub ? (
                <span className="text-dark-tertiary">STUB · v0</span>
              ) : layer.red ? (
                <span className="text-carry-red">RED</span>
              ) : (
                <span className="text-senior">GREEN</span>
              )}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────
// Decision ticker
// ──────────────────────────────────────────────────────────────────────────

function DecisionTicker({ decisions }: { decisions: DecisionEvent[] }) {
  return (
    <div className="mt-4 rounded-xl border border-dark-border bg-black/30 p-3">
      <div className="flex items-center justify-between">
        <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-dark-secondary">
          decision log · dry-run
        </p>
        <span className="font-mono text-[10px] text-dark-tertiary">
          {decisions.length} events
        </span>
      </div>
      <ul className="mt-2 h-[120px] space-y-1 overflow-hidden font-mono text-[11px]">
        <AnimatePresence initial={false}>
          {decisions.map((d) => (
            <motion.li
              key={d.id}
              initial={{ opacity: 0, y: -6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.25 }}
              className="flex items-start gap-2 leading-snug"
            >
              <span className="shrink-0 text-dark-tertiary">
                T+{(d.tsSimMs / 1000).toFixed(2)}s
              </span>
              <span className={`shrink-0 ${kindColor(d.kind)}`}>
                [{d.kind}]
              </span>
              <span className="truncate text-dark-primary">{d.message}</span>
            </motion.li>
          ))}
        </AnimatePresence>
      </ul>
    </div>
  );
}

function kindColor(k: DecisionEvent["kind"]): string {
  switch (k) {
    case "open":
      return "text-senior";
    case "cycle_lock":
      return "text-carry-amber";
    case "rebalance":
      return "text-pacific-400";
    case "stub":
      return "text-dark-tertiary";
    default:
      return "text-dark-secondary";
  }
}

// ──────────────────────────────────────────────────────────────────────────
// Footnote
// ──────────────────────────────────────────────────────────────────────────

function ConsoleFootnote() {
  return (
    <p className="mt-4 border-t border-dark-border/60 pt-3 font-mono text-[10px] leading-relaxed text-dark-tertiary">
      Week-1 scope honesty: Pacifica funding is the only live venue; Backpack /
      Hyperliquid / Lighter are fixtures wired to exercise the 4-venue pipeline
      end-to-end. The 1892 bps spread is a fixture artifact — a full 4-venue
      live run would see a smaller effective spread as cross-venue arb closes.
      Every decision is dry-run; RUNNER_ALLOW_LIVE is off. Framework layers
      forecast_scoring / risk_stack / fsm are intentionally stubbed for Week-1.
    </p>
  );
}
