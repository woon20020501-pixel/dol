# bot-rs

Rust trading engine for the Dol cross-venue funding harvester. Reads Pacifica + counter-venue market data, emits per-tick signal JSON and an `nav.jsonl` audit trail, and is fronted by a preflight gate that blocks live order submission until the v0 safety-component checklist is green.

**The demo path is read-only "would-have-executed" telemetry.** No code path in the workspace submits a live order today; the live-submission gate in `bot-runtime::live_gate` is an intentional tripwire, not a stub.

## Purpose

One-paragraph contract:

The bot fetches snapshots from four venues (Pacifica, Hyperliquid, Lighter, Backpack) each tick, computes a per-symbol fair value, picks the best `(long_venue, short_venue)` spread that clears the cost model and the funding-cycle lock, accrues NAV with a one-time round-trip cost + pure funding income on hold, and writes a signed audit record per symbol per tick. The iron law — same symbol, two DEX venues, opposite directions, funding-only revenue, zero price exposure — is enforced as a type-level invariant (`Venue` closed enum, `PairDecision` with one `symbol` field and two venue refs) and as a runtime invariant (`funding_cycle_lock::enforce` on every decision).

## Workspace

```
bot-rs/
├── Cargo.toml                 resolver=2, rust-version=1.75, LTO=thin
├── crates/
│   ├── bot-types              Newtypes, Venue enum, Mandate, LiveInputs, OuParams, FrameworkError
│   ├── bot-math               Pure primitives — phi, OU avg, Bernstein, MFG, cap-routing, slippage, cost model, CVaR
│   ├── bot-strategy-v3        Statistical layer — funding_cycle_lock, fsm_controller, stochastic (fit_ou, adf, cvar), lock_typestate
│   ├── bot-venues             Low-level Pacifica + Lighter REST/WS clients; raw JSON parsing; reconnect
│   ├── bot-adapters           VenueAdapter trait + PacificaReadOnly, PacificaAuthenticated, PacificaSign, DryRun; chaos harness; rate limit; execution scaffold
│   ├── bot-runtime            Tick engine, NAV tracker (per-symbol + PortfolioNav aggregate), cycle-lock registry, adapter-health telemetry, signal emitter, live-gate preflight, scoring, history, metrics, risk stack scaffold
│   ├── bot-cli                `bot-rs demo` subcommand — the end-to-end loop
│   └── bot-tests              Python parity harness — loads strategy/rust_fixtures/*.json
└── docs/                     internal component-wiring status
```

### Crate dependencies

| Crate | Async? | Role |
|---|---|---|
| `bot-types` | sync | Newtypes (`AnnualizedRate`, `HourlyRate`, `Usd`, `AumFraction`, `Hours`, `Dimensionless`), `Venue` closed enum (4 variants), `Mandate`, `OuParams`, `ImpactParams`, `FrameworkError` |
| `bot-math` | sync | `phi`/`phi_derivative` (via `expm1` for numerical stability), `ou_time_averaged_spread`, `effective_spread_with_impact`, `break_even_hold_*`, `bernstein_leverage_bound`, `mfg_competitor_count`, `cap_routing` + `mandate_floor`, `slippage` (sqrt impact), `round_trip_cost_model_c`, `cvar::{cvar_empirical, cvar_ru}` |
| `bot-strategy-v3` | sync | `funding_cycle_lock::enforce` (direct Python port, parity-pinned), `fsm_controller::step` (5-axis FSM + Banach-damping clip), `stochastic::{fit_ou, adf_test, cvar_drawdown_stop, fit_drift, expected_residual_income}`, `lock_typestate` (compile-time invariant the lock is threaded through every decision) |
| `bot-venues` | async | Pacifica REST (`info/prices`, `book`, `funding_rate/aggregated`), Pacifica WS, Lighter REST + WS; exponential-backoff reconnect |
| `bot-adapters` | async | `VenueAdapter` trait; `PacificaReadOnlyAdapter`, `PacificaAuthenticatedAdapter` (credential redaction in `Debug`), `PacificaSign` (EIP-191 payload builder for the vault's `reportNAV`), `DryRunVenueAdapter` (fixture replay), `chaos` (injection of latency, timeout, partial response for resilience tests), `rate_limit`, `execution` (scaffold only — no live submission path) |
| `bot-runtime` | async | `TickEngine::run_one_tick` (7-stage pipeline, see below), `PortfolioNav`, `NavTracker`, `CycleLockRegistry`, `AdapterHealthRegistry`, `SimulatedClock`, `Scorer` (symbol scoring), `HistoryRing`, `metrics` (Prometheus-format), `risk::{cvar_budget, heartbeat, kill_switch, watchdog, drawdown, concentration}` (structural, not yet wired into `preflight_live_gate`) |
| `bot-cli` | async | `bot-rs demo` (subcommand: `cmd/demo.rs`), authenticated live fetch, continuous loop, time-acceleration |
| `bot-tests` | sync | Fixture loader + `Case<I, E>` harness; resolves `strategy/rust_fixtures/` via `CARGO_MANIFEST_DIR` walk-up or `DOL_FIXTURES_DIR` override |

### Per-tick pipeline (`TickEngine::run_one_tick`)

```
1. fetch_snapshot          parallel, per venue, per symbol (tokio::join_all)
2. compute_fair_value      depth-weighted mid; drops venues with is_healthy=false
3. decision::decide        score candidates, hysteresis gate, no-rebalance-while-held policy
4. CycleLockRegistry       funding_cycle_lock::enforce — held / opened_new_cycle / blocked
5. NavTracker::accrue      one-time round-trip cost + pure funding income on hold
6. AdapterHealthRegistry   per-symbol rolling failure counter, DEGRADATION_THRESHOLD=3 advisory flag
7. emit_signal             atomic write: output/signals/{symbol}/{yyyymmdd}/{ts_ms}.json
   nav.jsonl append        (one NavPoint per symbol + one AggregateNavPoint)
```

## Key interfaces

### `VenueAdapter` trait

```rust
#[async_trait]
pub trait VenueAdapter: Send + Sync {
    async fn fetch_snapshot(&self, symbols: &[&str]) -> Result<Vec<VenueSnapshot>, AdapterError>;
    async fn fetch_positions(&self) -> Result<Vec<PositionView>, AdapterError>;
    async fn submit_dryrun(&self, intent: OrderIntent) -> Result<FillReport, AdapterError>;
    fn venue(&self) -> Venue;
}
```

`submit_dryrun` is the only submission path implemented. Every adapter returns `FillReport { dry_run: true, ... }`. A live submission method does not exist on this trait.

### `funding_cycle_lock::enforce`

```rust
pub fn enforce(
    state: &mut Option<CycleState>,
    now: f64,
    proposed_h: i8,
    proposed_n: f64,
    emergency_override: bool,
    cycle_seconds: i64,
) -> EnforceResult  // { h_eff, n_eff }
```

Priority-ordered branches: emergency override → open new cycle → hold locked. Byte-for-byte port of `strategy/strategy/funding_cycle_lock.py`; parity-pinned to `strategy/output/rust_parity/funding_cycle_lock_fixtures.json`.

### Live submission preflight

`bot-runtime::live_gate::preflight_live_gate()` is called once in `bot-cli::main`. When `RUNNER_ALLOW_LIVE != "1"` it silently passes (demo mode). When `RUNNER_ALLOW_LIVE == "1"` it enumerates missing components from six `const fn has_*()` probes and fails closed.

Current status (wired in the recent Aurora-Ω integration):

| Component | `has_*()` | Status |
|---|---|---|
| `funding_cycle_lock` I-LOCK | `has_funding_cycle_lock` | **true** |
| `fsm_controller` emergency flatten (I-KILL) | `has_fsm_emergency_flatten` | **true** |
| `cvar_guard` non-stub (I-BUDGET) | `has_cvar_guard_nonstub` | **true** |
| `kill_switch` (SIGINT + file-flag) | `has_kill_switch` | **true** |
| Hedge `heartbeat` (5 s watchdog) | `has_heartbeat` | **true** |
| Pacifica API watchdog (3 s latency) | `has_pacifica_watchdog` | **true** |

Component wiring is compile-time (each `has_*` is `const fn`) — operator environment cannot flip a component to green without a rebuild.

## Signal JSON schema (abbreviated)

```json
{
  "version": "aurora-omega-1.0",
  "ts_unix": 1790012230.08,
  "symbol": "BTC",
  "fair_value": { "p_star": 100013.22, "healthy": true,
                  "contributing_venues": ["Backpack","Lighter","Hyperliquid","Pacifica"] },
  "cycle_lock":  { "locked": true, "cycle_index": 497225, "h_c": 1, "N_c": 100.07,
                   "opened_new_cycle": true, "proposed_was_blocked": false },
  "forecast_scoring": { "_stub": true, ... },
  "risk_stack":  [ { "layer": "cvar", "red_flag": false, "_stub": true }, ... ],
  "fsm":         { "mode": "kelly_safe", "notional_scale": 1.0, "_stub": true },
  "diagnostics": {
    "pacifica_authenticated": true,
    "oracle_divergence_risk": "structural" | "minimal",
    "book_parse_failures": { "consecutive_failures": 0, "is_degraded": false }
  },
  "extra": { "pair_decision": { "long_venue": "Pacifica", "short_venue": "Backpack",
                                "symbol": "BTC", "notional_usd": 100.0,
                                "would_have_executed": true } }
}
```

Fields flagged `_stub: true` are present in the schema but return reference values that downstream consumers must treat as informational (see `diagnostics.stubbed_sections` for the enumeration).

`oracle_divergence_risk = "structural"` is honest disclosure for RWA pairs where Pacifica and Hyperliquid may reference different spot oracles (XAU, XAG, PAXG). Crypto pairs return `"minimal"`.

## Dependencies

| Crate | Version | Use |
|---|---|---|
| `tokio` | 1.40 (features=`full`) | async runtime |
| `reqwest` | 0.12 (features=`json, native-tls`) | REST client for adapters and venues |
| `tokio-tungstenite` | 0.24 | WS client for Pacifica/Lighter streaming |
| `serde` / `serde_json` | 1 | signal JSON, fixture I/O |
| `tracing` + `tracing-subscriber` | 0.1 / 0.3 | structured logging |
| `anyhow` / `thiserror` | 1.0 | error propagation |
| `hashbrown` | 0.14 | faster `HashMap` where workload is CPU-bound |
| `proptest` | 1 | property tests (`bot-math/tests/proptests.rs`) |
| `async-trait` | 0.1 | the `VenueAdapter` trait |
| `chrono` | 0.4 | timestamps |

## Testing

```bash
cd bot-rs
cargo test --workspace
```

Current: **411 passed, 0 failed, 11 ignored.**

Coverage by suite:

| Crate / target | Tests | Focus |
|---|---|---|
| `bot-adapters` unit | 31 | Pacifica JSON shape, dryrun fixture loading, rate-limit wheel, chaos injection, credential redaction in `Debug` |
| `bot-adapters::dryrun_smoke` | 6 | 4-venue snapshot load + symbol-not-found surface |
| `bot-adapters::pacifica_auth_unit` | 9 | `from_env` missing/present paths; redaction in `Display`/`Debug` |
| `bot-adapters::pacifica_auth_live` | 3 (`#[ignore]`) | Live API with `PACIFICA_API_KEY` — run via `-- --ignored` |
| `bot-adapters::pacifica_live` | 4 (`#[ignore]`) | Live REST surface shape |
| `bot-math` unit | 65 | Every primitive — boundary values, reference-implementation sanity checks |
| `bot-math::proptests` | 17 | Property tests: slippage monotonicity, φ invariants, HHI bounds, CVaR monotonicity (Rockafellar-Uryasev Theorem 1), cap-routing conservation (∑ slices = gross), Bernstein monotonicity |
| `bot-runtime` unit | 121 | `NavTracker` accrue across all 5 `PositionEvent` branches, `PortfolioNav` aggregate arithmetic, `CycleLockRegistry`, `AdapterHealthRegistry` degradation trip, `SimulatedClock` affine-transform, `live_gate` checklist, `decision::decide` hysteresis + no-rebalance, scoring, history ring, metrics exposition |
| `bot-runtime::tick_smoke` | 3 | multi-symbol tick smoke, PortfolioNav correction, per-venue dryrun fixtures |
| `bot-runtime::pacifica_live_tick` | 1 (`#[ignore]`) | Real Pacifica tick end-to-end |
| `bot-strategy-v3` unit | 42 | `funding_cycle_lock` enforce/open/is_locked/would_violate_lock × every priority branch; `fsm_controller` Kelly/Neutral/Robust transitions + Banach clip; `stochastic` OU MLE recovery + ADF + CVaR + drift; `lock_typestate` compile-fail tests |
| `bot-tests::parity_math` | 8 | `bot-math` primitives against Python fixtures at per-case tolerance |
| `bot-tests::parity_stochastic` | passes in per-module iteration | OU MLE, ADF, CVaR drawdown, hurst |
| `bot-tests::parity_funding_cycle_lock` | 1 | 28 fixture cases covering every public function |
| `bot-tests::parity_phase1` | 10 | Phase-1 integration surface |
| `bot-tests::parity_phase2a` | Phase 2 gated | — |
| `bot-tests::strategy_comparison` | — | Reserved |
| `bot-types` unit | 1 | Newtype sanity |
| `bot-venues` unit | 3 | Config, reconnect backoff |
| `bot-venues::live_ws` | 0 (`#[ignore]`) | Live WS — run manually |
| `bot-venues::mock_ws` | 1 | Mock WS roundtrip |

Ignored tests require live API credentials or network access; run with `cargo test -- --ignored --nocapture`.

## Running the demo

```bash
cd bot-rs
cargo build --release --workspace

# Authenticated live fetch (read-only), 3600× time acceleration
export PACIFICA_API_KEY=<your_api_key>
export PACIFICA_BUILDER_CODE=<your_builder_code>

cargo run --release --bin bot-rs -- demo \
  --continuous \
  --tick-interval-secs 2 \
  --starting-nav 10000 \
  --pacifica-live \
  --pacifica-auth \
  --accel-factor 3600 \
  --dryrun-fixtures crates/bot-adapters/tests/fixtures/dryrun \
  --signal-dir output/demo_smoke/signals \
  --nav-log output/demo_smoke/nav.jsonl
```

- `--pacifica-live` hits the real `api.pacifica.fi/api/v1/info/prices` and `/book` for all 10 symbols.
- `--pacifica-auth` adds `X-API-Key` + `X-Builder-Code` headers so account/builder-status endpoints are accessible and each signal JSON gets `diagnostics.pacifica_authenticated: true`.
- `--accel-factor 3600` maps 1 wall second to 1 simulated hour for visible NAV animation.
- Ctrl-C cleanly flushes the NAV log and exits.

**`RUNNER_ALLOW_LIVE=1` is not enough to turn on live submission.** There is no live-submission code path in the workspace today; the preflight gate is an explicit future-proofing tripwire.

## Integration points

- **Contracts** — the bot holds the `OPERATOR_ROLE` key. The EIP-191 payload builder for `reportNAV(value, signature)` lives in `bot-adapters::pacifica_sign`; the vault side is `contracts/packages/contracts/src/PacificaCarryVault.sol::reportNAV`, with a byte-for-byte golden-vector test at `test/PacificaCarryVault.navReport.t.sol`.
- **Dashboard** — consumes `output/demo_smoke/{nav.jsonl, signals/}` through the dashboard's `/api/nav` and `/api/signal` server routes (30-second staleness threshold, snapshot fallback).
- **Strategy (Python reference)** — parity fixtures consumed from `../strategy/rust_fixtures/`. New math primitives land in Python first, then a Rust port + a new fixture drop; `bot-tests` enforces the bit-level agreement before any commit crosses.

## Ship status

| Surface | Status |
|---|---|
| Per-tick pipeline (fetch → decide → accrue → emit) | Working end-to-end with live Pacifica + fixture-served counter-venues |
| Pacifica read-only adapter | Live; ~150 ms wall-clock round trip |
| Pacifica authenticated adapter | Live; credential redaction verified |
| Hyperliquid / Lighter / Backpack live adapters | Fixture-served in the demo; live adapters are a Week-2 task |
| `funding_cycle_lock` I-LOCK gate | Shipped + parity-pinned to Python |
| `fsm_controller` I-KILL | Shipped; 9 unit tests covering Kelly/Neutral/Robust + Banach clip |
| `cvar_budget` I-BUDGET (non-stub) | Shipped under `risk::cvar_budget` |
| Kill switch + heartbeat + Pacifica watchdog | Shipped under `risk::{kill_switch, heartbeat, watchdog}` |
| Live order submission path | **Not implemented.** Preflight gate blocks. |
| `fair_value_oracle` full Kalman 2-state + staleness weighting | Simplified depth-weighted mean shipped; full version Week 2+ |
| `forecast_scoring` α-cascade | Stubbed in signal JSON (`_stub: true`) |
| Slippage live calibration | Python reference shipped; Rust port Week 2+ |
| `45-FMA / 300 ns` fast-path decision kernel | Not implemented — post-demo optimization |
