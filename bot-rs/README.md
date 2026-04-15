# bot-rs — Dol Cross-Venue Funding Harvester

**A Rust trading engine that captures the funding-rate spread between two perpetual DEXes while holding the same asset on both sides, so net price exposure is zero by construction.**

This repository is a decision + telemetry engine for the Dol strategy. It reads live market data from Pacifica and three counter-venues, enforces the iron law ("same asset, two venues, opposite sides, funding-only revenue") as a type-level invariant in Rust, computes per-symbol decisions through a hysteresis-gated funding-cycle lock, and writes an append-only audit trail of every action to disk. It does NOT currently submit orders — the demo path is read-only "would have executed" telemetry, with an explicit startup preflight that blocks any live-submission path until a documented set of v0 safety components are wired.

The implementation was built for the Pacifica hackathon demo. The math layer ports primitives from a Python reference framework (Aurora-Ω / Dol v3.5.2, maintained separately) and is unit-tested to bit-level parity against Python fixtures where a parity harness is in place. The runtime layer wires those primitives into a tick-driven loop over a user-configurable universe (currently 10 symbols: 7 crypto + 3 RWA).

---

## Table of contents

1. [The iron law (why this is safe)](#the-iron-law)
2. [Architecture](#architecture)
3. [Quick start — build, test, demo](#quick-start)
4. [What's implemented vs what's deferred](#whats-implemented)
5. [Signal JSON output schema](#signal-json-output-schema)
6. [Key files for a reviewer](#key-files-for-a-reviewer)
7. [Tests](#tests)
8. [Honest limitations](#honest-limitations)
9. [References to the strategy framework](#references-to-the-strategy-framework)

---

## The iron law

From `PRINCIPLES.md §1` of the Aurora-Ω framework (the single non-negotiable rule the bot must preserve):

> **Dol earns yield by holding the same asset on two perpetual DEX venues in opposite directions, capturing the funding rate spread, with zero net price exposure.**

Concretely:
- Long instrument X on venue A, short instrument X on venue B
- A and B are two distinct DEXes from the four-venue whitelist: **Pacifica, Hyperliquid, Lighter, Backpack**
- X is the literally identical symbol on both sides — `BTC-PERP` vs `BTC-PERP`, not `cbBTC` vs `WBTC`
- Revenue source is the funding-rate differential only — never directional P&L, never statistical pair trades, never cross-asset arbitrage
- No CEX counter-venue exposure (no KYC, no custodial risk)

The iron law is enforced at **four walls** in the code:

| Wall | Invariant | Enforcement point |
|---|---|---|
| 1 | **I-LOCK** — every direction decision traces to a `funding_cycle_lock::enforce` call in the same tick | `bot-strategy-v3::funding_cycle_lock` + `bot-runtime::cycle_lock::CycleLockRegistry::enforce_decision` |
| 2 | **I-VENUE** — order targets are restricted to the 4-DEX whitelist, type-checked | `bot_types::Venue` closed enum (4 variants, no fallthrough) |
| 3 | **I-SAME** — both legs of a pair have byte-identical symbol strings | `PairDecision { long_venue, short_venue, symbol }` — one symbol field, two venue refs |
| 4 | **I-KILL** — FSM emergency flatten → cancel + IOC flatten within 120 seconds (deferred to v1+) | `bot-runtime::live_gate` preflight blocks live submission until `fsm_emergency_flatten_wired = true` |

See `strategy/docs/integration-spec.md §4` for the full 11-invariant status table.

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              bot-cli (demo)                              │
│   bot-rs demo --pacifica-live --pacifica-auth --continuous --accel 3600  │
└──────────────────────────────┬───────────────────────────────────────────┘
                               │
                ┌──────────────▼──────────────┐
                │       bot-runtime            │  ← tick engine, NAV tracker,
                │  TickEngine / SimulatedClock │    cycle-lock registry,
                │  NavTracker / PortfolioNav   │    adapter health telemetry,
                │  CycleLockRegistry           │    signal JSON emitter,
                │  AdapterHealthRegistry       │    live-submission preflight
                │  live_gate / signal          │
                │  decision / fair_value       │
                └──┬───────────────┬───────────┘
                   │               │
         ┌─────────▼────────┐  ┌───▼──────────────────┐
         │   bot-adapters    │  │   bot-strategy-v3     │  ← framework layer
         │                   │  │                        │
         │ VenueAdapter trait│  │  funding_cycle_lock    │  (I-LOCK gate,
         │ PacificaReadOnly  │  │  stochastic: fit_ou,   │   Python parity)
         │ PacificaAuthentd. │  │  adf, cvar, drift, …   │
         │ DryRunVenueAdapter│  └──┬─────────────────────┘
         └──┬─────────────────┘    │
            │                      │
            │              ┌───────▼─────┐
            │              │  bot-math    │  ← pure math primitives
            │              │              │    phi, Bernstein, MFG,
            │              │              │    cap routing, OU avg, …
            │              └───────┬──────┘
            │                      │
    ┌───────▼───────┐     ┌────────▼─────┐
    │  bot-venues    │     │  bot-types    │  ← newtypes, Venue enum,
    │                │     │               │    Mandate, OuParams,
    │ pacifica/rest  │     │               │    FrameworkError
    │ pacifica/ws    │     └───────────────┘
    │ lighter/rest   │
    │ lighter/ws     │         ┌───────────┐
    └────────────────┘         │ bot-tests  │  ← parity harness,
                               │            │    loads
                               │            │    ../strategy/
                               │            │    rust_fixtures/*.json
                               └────────────┘
```

### Data flow per tick (`TickEngine::run_one_tick`)

```
1. fetch_snapshot (parallel, per venue, per symbol)
        ↓
2. compute_weighted_fair_value (drops unhealthy venues)
        ↓
3. decision::decide (picks best-spread pair + no-rebalance hysteresis)
        ↓
4. CycleLockRegistry::enforce_decision (funding_cycle_lock.enforce)
        ↓ (held / opened_new_cycle / blocked)
5. NavTracker::accrue (income − one-time entry cost)
        ↓
6. AdapterHealthRegistry::record_tick (per-symbol degradation telemetry)
        ↓
7. emit_signal → output/signals/{symbol}/{yyyymmdd}/{ts}.json   ← audit trail
   nav.jsonl append                                               ← dashboard stream
```

### Workspace layout

| Crate | Role | Type |
|---|---|---|
| `bot-types` | Newtypes (`AnnualizedRate`, `HourlyRate`, `Usd`, `AumFraction`), `Venue` enum, `FrameworkError`, `Mandate` / `LiveInputs` / `OuParams` / `ImpactParams` structs | Pure, no async |
| `bot-math` | Pure math primitives ported from the Aurora-Ω Python reference: `phi` (absorption function), `ou_time_averaged_spread`, `bernstein_leverage_bound`, `mfg_competitor_count`, `cap_routing`, `round_trip_cost_model_c`, … | Pure, parity-tested |
| `bot-strategy-v3` | Statistical layer: `funding_cycle_lock` (I-LOCK enforcement, direct Python port), `stochastic::fit_ou`, ADF test, CVaR drawdown, expected residual income, drift fit | Pure |
| `bot-tests` | Python parity test harness. Loads JSON fixtures from `../strategy/rust_fixtures/` and asserts bit-level agreement on the math primitives | Pure, test-only |
| `bot-venues` | Low-level Pacifica + Lighter WS/REST clients. Raw JSON parsing, reconnect logic, URL config | Async (tokio + reqwest + tokio-tungstenite) |
| `bot-adapters` | `VenueAdapter` trait + wrappers: `PacificaReadOnlyAdapter`, `PacificaAuthenticatedAdapter` (authenticated read + builder code attribution), `DryRunVenueAdapter` (fixture replay) | Async |
| `bot-runtime` | Tick engine, NAV tracker (`NavTracker` / `PortfolioNav`), cycle-lock registry, adapter health telemetry, simulated clock, signal JSON emitter, live submission preflight gate | Async |
| `bot-cli` | `bot-rs demo` subcommand — the end-to-end tick loop with CLI flags for authentication, continuous mode, time acceleration, output paths | Async (tokio::main) |

---

## Quick start

### Prerequisites

- Rust 1.75 or later (stable toolchain)
- A Pacifica builder account if you want to run the authenticated adapter (optional — public-REST read works without)

### Build

```bash
cd bot-rs
cargo build --release --workspace
```

### Run the full test suite

```bash
cargo test --workspace
```

All tests passing, no failures.

### Run the demo (authenticated, continuous, 3600× time acceleration)

```bash
# PowerShell
$env:PACIFICA_API_KEY='<your_api_key>'
$env:PACIFICA_BUILDER_CODE='<your_builder_code>'

cargo run --release --bin bot-rs -- demo `
  --continuous `
  --tick-interval-secs 2 `
  --starting-nav 10000 `
  --pacifica-live `
  --pacifica-auth `
  --accel-factor 3600 `
  --dryrun-fixtures crates/bot-adapters/tests/fixtures/dryrun `
  --signal-dir output/demo_smoke/signals `
  --nav-log output/demo_smoke/nav.jsonl
```

- `--continuous` runs until Ctrl-C (SIGINT cleanly flushes the NAV log and exits)
- `--accel-factor 3600` maps 1 real second to 1 simulated hour for visible NAV curve animation
- `--pacifica-live` hits the real `https://api.pacifica.fi/api/v1/info/prices` and `/book` endpoints for all 10 symbols
- `--pacifica-auth` adds the `X-API-Key` + `X-Builder-Code` headers so account / builder-status endpoints are accessible and every signal JSON gets `diagnostics.pacifica_authenticated: true` + `diagnostics.builder_code`

**Important**: the demo path never submits an order. The live submission gate is at `bot-runtime::live_gate::preflight_live_gate`, which refuses to let the bot start in live-submit mode unless the v0 component wiring checklist is all green. The current checklist has 5 of 6 items still unwired, so the preflight intentionally fails if you ever set `RUNNER_ALLOW_LIVE=1` before those components land.

### Tail the live signal stream

```bash
tail -f output/demo_smoke/nav.jsonl
```

Each line is one `NavPoint` (per-symbol) or one `AggregateNavPoint` — see `bot-runtime::nav::NavPoint` / `AggregateNavPoint` for the struct shapes, both `#[derive(Serialize)]`.

---

## What's implemented

### Runtime layer (`bot-runtime` + `bot-cli`)

- **`TickEngine`** — parallel venue fetch (`tokio::join_all`), per-adapter graceful degradation, drop-and-continue on transient API failures
- **`PortfolioNav`** — per-symbol NAV tracker with aggregate rollup row; each per-symbol tracker is initialized at the full portfolio NAV so the 1%-of-NAV notional rule yields the correct $100/pair rather than a sliced $10/pair
- **`NavTracker`** — one-time round-trip entry cost + pure funding income on hold (10-bp cost stub, configurable); explicit `PositionEvent { Idle / Opened / Held / Rebalanced / HeldThroughGap }` variants
- **`CycleLockRegistry`** — thread-through of `funding_cycle_lock::enforce` for the iron-law direction gate, with per-symbol state and locked-pair-identity tracking for rebalance detection
- **`AdapterHealthRegistry`** — per-symbol consecutive / total failure counters, `DEGRADATION_THRESHOLD = 3` advisory flag surfaced into signal JSON `diagnostics.book_parse_failures`
- **`SimulatedClock`** — affine-transform clock (`real_start_ms + (real_now − real_start) × factor`) for visual NAV animation without contaminating live API fetches with simulated time
- **Signal JSON emitter** — atomic write (temp file + rename), per-symbol + per-tick, `output/signals/{symbol}/{yyyymmdd}/{ts_ms}.json`
- **Live submission preflight gate** — env-var `RUNNER_ALLOW_LIVE=1` + component-wiring checklist; fails closed

### Strategy framework (`bot-strategy-v3`)

- **`funding_cycle_lock`** — direct port of the Python `strategy/funding_cycle_lock.py`, enforces the I-LOCK invariant. `enforce` has three priority-ordered branches (emergency override → open new cycle → hold locked) and is exhaustively unit-tested
- **`stochastic::fit_ou`** — Ornstein-Uhlenbeck MLE via exact discretization AR(1) regression
- **`stochastic::adf_test`** — Augmented Dickey-Fuller test, hand-rolled (no scipy dependency)
- **`stochastic::cvar_drawdown_stop`** — empirical CVaR drawdown-stop calculation
- **`stochastic::expected_residual_income`** — expected future spread income under OU dynamics
- **`stochastic::fit_drift`** — drift-regime alternative when Hurst > 0.70

### Math primitives (`bot-math`)

All ported from the Aurora-Ω / Dol v3.5.2 Python reference with bit-level parity tests against committed fixtures:

- **`phi`** — absorption function `(1 − e^(−x)) / x`, with `expm1`-based numerical stability (catches a classic cancellation bug at small x)
- **`phi_derivative`** — analytic derivative with Taylor fallback near zero
- **`ou_time_averaged_spread`** — OU mean-reverting expected spread over a hold
- **`effective_spread_with_impact`** — OU spread net of orderbook impact and competition
- **`break_even_hold_at_mean`** / **`break_even_hold_fixed_point`** — closed-form + iterative break-even hold time
- **`optimal_margin_fraction`** / **`optimal_notional`** / **`optimal_trading_contribution`** — interior-optimum sizing
- **`critical_aum`** — AUM threshold for the decay-binding regime
- **`bernstein_leverage_bound`** — robust leverage bound from Bernstein concentration
- **`mfg_competitor_count`** — free-entry Nash equilibrium competitor count
- **`dol_sustainable_flow_per_pair`** — sustainable-edge flow calculation
- **`capacity_ceiling`** — vault capacity ceiling
- **`cap_routing`** / **`mandate_floor`** — customer/buffer/reserve cap routing with conservation invariant
- **`slippage`** — square-root impact slippage (Almgren-Chriss style)
- **`round_trip_cost_model_c`** — Model C (maker/taker asymmetric) round-trip cost

### Venue adapters (`bot-adapters` + `bot-venues`)

- **`PacificaReadOnlyAdapter`** — parallel `/info/prices` + `/book` fetch via `tokio::try_join!`, ~150 ms wall-clock round trip
- **`PacificaAuthenticatedAdapter`** — wraps the read-only adapter, adds `X-API-Key` / `X-Builder-Code` headers, credential redaction in the `Debug` impl, env-var-only secret handling
- **`DryRunVenueAdapter`** — loads fixture JSON snapshots for offline / fixture-replay testing
- **Dry-run order submission path** — every adapter implements `submit_dryrun` which logs the "would have executed" intent and returns a synthetic `FillReport { dry_run: true }`. No live submission path exists anywhere in the workspace

### Deferred to v1+

- `fair_value_oracle::compute_fair_value` — full Kalman 2-state + staleness-weighting + `healthy` gate (current: simplified depth-weighted mean)
- `depth_threshold` + `fractal_delta` — shallow-venue cut and live impact exponent
- `forecast_scoring` — α-cascade tail monitor (BaselineRing + `tail_deterioration_flag`)
- `risk_stack` — 4-layer risk accounting (entropic CE, ECV, CVaR, execution χ²)
- `cvar_guard` with BUDGET_99 / BUDGET_95 — the deep + fast second-line-of-defense
- `hedge_ioc` — IOC certainty + failover ranking
- `partial_fill_model` — Beta(2,5) posterior + residual pool + Kaplan-Meier survival
- `toxicity_filter` — adverse-selection cancel gate
- `fsm_controller` — 5-axis FSM + self-correcting adapter + `emergency_flatten`
- `kill_switch` / `heartbeat` / Pacifica API watchdog — bot-owned guards
- `slippage_calibration` — live fill-to-model recalibration
- Real Hyperliquid / Lighter / Backpack live adapters — currently fixture-served in the demo
- 45-FMA / 300 ns fast-path decision kernel — Week 2+ post-demo optimization target

---

## Signal JSON output schema

Every tick writes one signal JSON per symbol to `output/signals/{symbol}/{yyyymmdd}/{ts_ms}.json`. Abbreviated sample:

```json
{
  "version": "aurora-omega-1.0",
  "ts_unix": 1790012230.08,
  "symbol": "BTC",
  "fair_value": {
    "p_star": 100013.22,
    "healthy": true,
    "contributing_venues": ["Backpack", "Lighter", "Hyperliquid", "Pacifica"],
    "total_weight": 145600.0
  },
  "cycle_lock": {
    "locked": true,
    "cycle_index": 497225,
    "h_c": 1,
    "N_c": 100.07,
    "seconds_to_cycle_end": 1585.92,
    "emergency_override": false,
    "opened_new_cycle": true,
    "proposed_was_blocked": false
  },
  "forecast_scoring": { "S_t": 0.0, "z": 0.0, "flag_fired": false, "_stub": true },
  "risk_stack": [
    { "layer": "entropic_ce",    "value": 0.0, "threshold": 0.02, "red_flag": false, "_stub": true },
    { "layer": "ecv",            "value": 0.0, "threshold": 0.05, "red_flag": false, "_stub": true },
    { "layer": "cvar",           "value": 0.0, "threshold": 0.05, "red_flag": false, "_stub": true },
    { "layer": "execution_chi2", "value": 0.0, "threshold": 15.0, "red_flag": false, "_stub": true }
  ],
  "fsm": { "mode": "kelly_safe", "notional_scale": 1.0, "emergency_flatten": false, "_stub": true },
  "orders": [],
  "single_venue_exposure": { "pacifica": 0.01, "backpack": 0.01 },
  "diagnostics": {
    "framework_commit": "demo",
    "bot_commit": "demo",
    "stubbed_sections": ["forecast_scoring", "risk_stack", "fsm"],
    "pacifica_authenticated": true,
    "builder_code": "huhcho",
    "oracle_divergence_risk": "minimal",
    "book_parse_failures": {
      "total_failures": 0,
      "consecutive_failures": 0,
      "is_degraded": false,
      "last_failure_venue": null
    }
  },
  "extra": {
    "pair_decision": {
      "long_venue": "Pacifica",
      "short_venue": "Backpack",
      "symbol": "BTC",
      "spread_annual": 0.2009,
      "notional_usd": 100.00,
      "reason": "hold pacifica/backpack spread=2009.84bps (demo: no-rebalance policy)",
      "would_have_executed": true
    },
    "nav_after": 10000.02,
    "demo_note": "..."
  }
}
```

**Stubbed sections** (`forecast_scoring`, `risk_stack`, `fsm`) are flagged with `_stub: true` per-field and enumerated in `diagnostics.stubbed_sections`. They'll be replaced with the real ported modules as `bot-strategy-v3` grows in Week 2+.

**`diagnostics.oracle_divergence_risk`** — `"structural"` for RWA symbols (XAU, XAG, PAXG — independent oracle on each leg), `"minimal"` for crypto symbols (same oracle abstraction on both legs). Honest tail-risk disclosure per the iron law `§5.1`.

**`diagnostics.book_parse_failures`** — per-symbol rolling adapter health telemetry. Flips `is_degraded: true` after `DEGRADATION_THRESHOLD = 3` consecutive ticks with at least one venue fetch failure. Advisory only in v0 (does NOT exclude the symbol from the decision loop — that's the v1+ retry-and-degrade policy).

---

## Key files for a reviewer

If you have 15 minutes, look at these in order:

1. **`crates/bot-strategy-v3/src/funding_cycle_lock.rs`** — the iron-law gate. Direct port from Python; a line-by-line comparison against the Python source would verify the control flow. 15+ unit tests covering every priority branch
2. **`crates/bot-runtime/src/cycle_lock.rs`** — the runtime wrapper that threads every decision through `funding_cycle_lock::enforce`. Includes a discussion of how the scalar `h_c` maps to `(long_venue, short_venue)` pairs via a canonical ordering, and the documented limitation of that mapping
4. **`crates/bot-runtime/src/tick.rs::TickEngine::run_one_tick`** — the 7-step tick pipeline, one function, reads top to bottom
5. **`crates/bot-runtime/src/live_gate.rs`** — the `RUNNER_ALLOW_LIVE=1` preflight gate. Fails closed; ships 6 component `const fn` checks; comments document the upgrade path from 6 to 22 (full v0 subset from `integration-spec.md §3.5`)
6. **`crates/bot-runtime/src/nav.rs`** — NAV tracker with one-time entry cost + pure funding income on hold. Comment blocks document the accounting model and the `PortfolioNav` correction (no sliced per-symbol NAV, use full portfolio NAV with aggregate rollup)
7. **`crates/bot-math/src/phi.rs`** — example of the parity-testing style. `phi(x) = (1 − e^(−x)) / x` via `expm1` for numerical stability, with a doc comment explaining the cancellation bug this guards against. Parity-tested against Python fixtures
8. **`crates/bot-adapters/src/pacifica_auth.rs`** — Pacifica authenticated adapter. Credential redaction in `Debug`, env-var-only secret handling, no order submission path, delegation pattern to the read-only adapter
9. **`output/demo_smoke/signals/BTC/*/*.json`** — an actual live signal JSON produced by the bot. Compare against the schema section above to verify every field is populated as documented

---

## Tests

The workspace covers:

- **Bot math primitives** — internal sanity + boundary + parity against Python fixtures
- **Strategy layer** (`bot-strategy-v3`) — `funding_cycle_lock` enforce / open / is_locked / would_violate_lock, OU MLE recovery on synthetic data, ADF test on known-stationary + known-non-stationary series, CVaR drawdown, drift fit recovery
- **Runtime** — `NavTracker` accrue model across all 5 `PositionEvent` branches, `PortfolioNav` aggregate arithmetic, `CycleLockRegistry` enforce outcomes, `AdapterHealthRegistry` degradation trip, `SimulatedClock` affine-transform correctness, `live_gate` component-wiring checklist, `decision::decide` hysteresis + no-rebalance policy
- **Venue adapters** — `DryRunVenueAdapter` fixture loading + symbol-not-found handling, `PacificaReadOnlyAdapter` snapshot shape, `PacificaAuthenticatedAdapter` credential redaction + `from_env` missing / present paths
- **Runtime integration smoke** — multi-symbol smoke test (10 symbols), 3-tick BTC smoke test, live Pacifica tick test (`#[ignore]` by default, run with `-- --ignored --nocapture` for manual verification)

To run parity tests only:

```bash
cargo test -p bot-tests
```

All parity fixtures live under `../strategy/rust_fixtures/` and are generated by the Python reference maintained in the sibling `strategy/` package. The fixture generator produces floats with explicit per-case tolerances (`1e-12` for exact arithmetic, `1e-6` for MLE outputs, `1e-9` for Cholesky solves).

---

## Honest limitations

Things a real trading firm would call out, and this engine does not yet address:

1. **No live order submission path**. Every adapter call is either read-only or `submit_dryrun`. The preflight gate in `bot-runtime::live_gate` refuses to let the bot start in live-submit mode until 6 components are wired, and currently only 1 of 6 is green (`funding_cycle_lock`).
2. **`fair_value_oracle` is simplified**. The current implementation is a depth-weighted mean of venue mids with a "≥ 2 contributing venues" health gate. The real Aurora-Ω `fair_value_oracle` has a Kalman 2-state lead/lag tracker, staleness weights, and a tier-based `healthy` flag that the bot does not yet consume.
3. **`risk_stack`, `forecast_scoring`, `fsm_controller` are stubbed in signal JSON**. Every field is serialized to `_stub: true` and the `diagnostics.stubbed_sections` array names each deferred section. No runtime behavior depends on these until they're ported.
4. **Demo-mode hardcodes**. The current bot runs at `min_spread_threshold = 2%` (raised from the framework's intended 2 bps to prevent churn at accel 3600) and has a "no rebalance once held" decision policy. Both are commented in `bot-runtime/src/decision.rs` as demo-only workarounds, not the intended production behavior. Both are removed before any Tier 1 live submission.
5. **The `45-FMA / 300 ns` fast-path kernel does not exist yet**. It's a Week 2+ optimization target with a design-sketch-first coordination step. The current decision path is on the order of microseconds per tick, not nanoseconds.
6. **Parity coverage is partial**. `bot-math` primitives are parity-tested against Python fixtures. `bot-strategy-v3::funding_cycle_lock` is ported but the 28-case Python-parity harness is pending (fixture file available, Rust test scaffolding Week 2 day 1). `bot-strategy-v3::stochastic` (`fit_ou` / `adf` / `cvar` / …) is tested with synthetic recovery but not yet parity-pinned to Python.
7. **Hyperliquid, Lighter, Backpack are fixture-served in the demo**. A real `HyperliquidAdapter` is a Week 2 task. The demo uses `DryRunVenueAdapter` backed by committed fixtures for those three venues, with live data only from Pacifica. This is documented in the demo runbook and visible in each signal JSON's `fair_value.contributing_venues` field.
8. **RWA (XAU/XAG/PAXG) pairs carry structural oracle divergence risk**. Flagged honestly in `diagnostics.oracle_divergence_risk: "structural"`. Pacifica's XAU oracle and Hyperliquid's `xyz:GOLD` HIP-3 oracle may use different references — the bot does not yet introspect either side's oracle metadata at startup. Post-demo research task.

All of these are tracked internally with explicit "fix before tier N" priority tags.

---

## References to the strategy framework

The math layer here ports from the Python reference maintained in the sibling `strategy/` package:

- `strategy/PRINCIPLES.md` — the iron law (`§1`) and anti-patterns (`§2`). Immutable contract.
- `strategy/docs/integration-spec.md` — the operational contract between the framework and any bot implementation, including the signal JSON schema (`§5.2`) and the 11 invariants the bot must preserve (`§4`).
- `strategy/docs/aurora-omega-spec.md` — the 33-section Aurora-Ω master spec.
- `strategy/docs/math-aurora-omega-appendix.md` — Appendix F (α-cascade strict propriety proof) + appendices for the other math modules.
- `strategy/rust_fixtures/*.json` — the parity test fixture set (22 files, ~131 cases across `phi`, `ou_time_averaged_spread`, `bernstein_leverage_bound`, `cap_routing`, …).
- `strategy/scripts/validate_aurora_omega.py` — the Python framework's own validation suite.

The math primitive layer (`bot-math`) is tested against the framework's fixture files at each build. When the framework updates a primitive, the Rust side updates in lockstep through the parity harness and a new fixture drop.

---

## License and attribution

Submitted for the Pacifica hackathon. Built on the Aurora-Ω / Dol v3.5.2 strategy framework maintained by the same team.
