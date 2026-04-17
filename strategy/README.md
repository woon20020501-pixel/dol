# strategy

Python reference framework for the Dol cross-venue funding harvester.

## Purpose

Authoritative specification of the math, gates, and decision rule that the Rust bot (`bot-rs`) implements and the Solidity contracts (`contracts`) rely on. This package is the single source of truth for:

- **Iron law** — same asset, two perpetual DEX venues, opposite directions, funding-rate spread only. Stated in `PRINCIPLES.md` and re-enforced at four walls in the Rust runtime.
- **Cost model** — fees, slippage (square-root impact), bridge, funding accrual.
- **Safety gates** — min-hold, persistence floor, OI cap, buffer floor, drawdown circuit breakers, funding-cycle lock.
- **Parity fixtures** — JSON test vectors (23 files) consumed by `bot-rs/crates/bot-tests` so the Rust port matches the Python reference bit-for-bit at f64 precision.

The framework never submits trades and never writes to chain. It produces signal JSON; a separate bot or operator acts on it.

## Layout

```
strategy/
├── PRINCIPLES.md                immutable iron law + anti-patterns
├── requirements.txt
├── strategy/                    package source
│   ├── cost_model.py            fees, slippage, bridge, funding accrual
│   ├── funding_cycle_lock.py    I-LOCK enforce/open/is_locked/would_violate_lock (185 lines; ported to Rust)
│   ├── fair_value_oracle.py     depth-weighted mid + ≥2-venue healthy gate (284 lines)
│   ├── fsm_controller.py        5-axis finite-state controller + emergency flatten (261 lines)
│   ├── risk_stack.py            entropic-CE / ECV / CVaR / execution-χ² 4-layer accounting (353 lines)
│   ├── forecast_scoring.py      α-cascade strict-propriety tail monitor (303 lines)
│   ├── stochastic.py            fit_ou (MLE via AR(1)), adf_test, cvar_drawdown_stop, hurst (458 lines)
│   ├── portfolio.py             cross-symbol allocation + mandate floor (390 lines)
│   ├── frontier.py              capacity ceiling, critical AUM, Bernstein leverage (478 lines)
│   ├── rigorous.py              closed-form break-even + optimal sizing (424 lines)
│   ├── slippage_calibration.py  live fill-to-model recalibration (353 lines)
│   ├── partial_fill_model.py    Beta(2,5) posterior + residual pool + Kaplan-Meier (235 lines)
│   ├── toxicity_filter.py       adverse-selection cancel gate (293 lines)
│   ├── hedge_ioc.py             IOC certainty + failover ranking (180 lines)
│   ├── latency_penalty.py       per-venue σ·√τ order-book penalty (162 lines)
│   ├── fractal_delta.py         live impact exponent δ̂ estimator (171 lines)
│   ├── lifecycle.py             AUM regime classification (285 lines)
│   ├── funding_bandit.py        ε-greedy counter-venue exploration (119 lines)
│   ├── depth_threshold.py       shallow-venue cut
│   ├── fallback_router.py       venue-outage reroute
│   ├── offset_controller.py     offset nudging
│   └── __init__.py
├── scripts/                     runners (not imported from the package)
│   ├── generate_rust_fixtures.py             produces rust_fixtures/*.json
│   ├── generate_funding_lock_parity_fixtures.py
│   ├── backtest_v2.py / backtest_v3_historical.py
│   ├── dry_run_v3_5.py                       end-to-end offline demo
│   ├── poll_aggregated.py                    live Pacifica funding-rate poller
│   ├── optimize_v3_5.py                      mandate param sweep
│   ├── validate_aurora_omega.py              full spec self-check
│   ├── validate_{formulas,frontier,rigorous,risk_budget,sprint2_3}.py
│   ├── analyze_persistence.py
│   ├── lifecycle_model.py
│   ├── merge_history.py
│   └── pull_external_history.py
├── tests/                       pytest suite
├── rust_fixtures/               JSON parity vectors (23 modules)
├── output/                      writable: backtest results, rust_parity/, live_funding.sqlite
└── docs/
    ├── integration-spec.md                  bot↔framework contract, 11 invariants, signal JSON schema
    ├── aurora-omega-spec.md                 master spec (the full Aurora-Ω design)
    ├── math-aurora-omega-appendix.md        appendix F (α-cascade propriety proof) + module appendices
    ├── math-derivation.md
    ├── math-formulas.md
    ├── math-frontier.md
    ├── math-rigorous.md
    └── pacifica-discovery.md                Phase-0 discovery of the Pacifica funding API
```

## Key modules

### `funding_cycle_lock.py` — I-LOCK gate

Direct source of truth for the Rust port at `bot-rs/crates/bot-strategy-v3/src/funding_cycle_lock.rs`. Exposes:

- `cycle_index(t, cycle_seconds) -> int`
- `cycle_phase(t, cycle_seconds) -> float`  (0.0 = cycle start, 1.0 = cycle end)
- `seconds_to_cycle_end(t, cycle_seconds) -> float`
- `is_locked(state, now) -> bool`
- `open_cycle(now, h_c, N_c, cycle_seconds) -> CycleState`
- `enforce(state, now, proposed_h, proposed_N, emergency_override, cycle_seconds) -> (h_eff, n_eff)` — three priority-ordered branches (emergency override → open new cycle → hold locked)
- `would_violate_lock(state, now, proposed_h) -> bool`

Parity-pinned via `rust_fixtures/funding_cycle_lock_fixtures.json` consumed by `bot-rs/crates/bot-tests/tests/parity_funding_cycle_lock.rs`.

### `cost_model.py` — explicit cost accounting

Concrete, non-symbolic costs for every income-vs-cost decision. Functions:

- `taker_fee(venue, notional)` → bps-based fee
- `slippage(notional, adv, impact_coef)` → Almgren-Chriss square-root
- `bridge_cost(src, dst, amount)` → cross-venue move cost
- `funding_accrual(rate_per_cycle, hold_cycles, notional)` → pure income on hold
- `round_trip(...)` → aggregate entry + exit cost

The explicit framing (vs. a symbolic "expected cost" regression) is the thing the bot consumes on every tick.

### `stochastic.py` — statistical primitives

- `fit_ou(x, dt)` — Ornstein-Uhlenbeck MLE via exact discretisation AR(1) regression
- `adf_test(x)` — Augmented Dickey-Fuller (hand-rolled, no scipy dependency)
- `cvar_drawdown_stop(returns, alpha)` — empirical CVaR drawdown-stop
- `hurst_exponent(x)` — rescaled-range Hurst estimate
- `expected_residual_income(...)` — expected future spread income under OU dynamics

### `rust_fixtures/` — parity vectors

23 JSON files (one per primitive module) with `(name, input, expected, tolerance)` case tuples. The Rust `bot-tests` crate loads these and asserts `|actual − expected| ≤ tolerance` per case. Per-module tolerances range from `1e-12` (exact arithmetic) through `1e-9` (Cholesky solves) to `1e-6` (MLE outputs).

Fixtures are regenerated by `scripts/generate_rust_fixtures.py` whenever the Python reference changes; the Rust side re-runs parity tests and must pass before any commit crosses to the bot repo.

## Dependencies

| Package | Purpose |
|---|---|
| `numpy`, `pandas` | numerical arrays, dataframes |
| `requests`, `aiohttp` | Pacifica REST + aggregated endpoint polling |
| `python-dotenv` | env-file loading |
| `pytest` | test runner |
| `playwright` | fallback polling path if Pacifica endpoints are JS-rendered (the discovery in `docs/pacifica-discovery.md` shows a JSON REST path exists, so playwright is rarely exercised) |

See `requirements.txt` for pinned minor versions.

## Testing

Run the full suite:

```bash
cd strategy
python -m venv .venv
.venv/Scripts/activate        # Windows
# or: source .venv/bin/activate
pip install -r requirements.txt
python -m pytest tests/ -q
```

Current result: **130 passed in ~0.2 s.**

| Test file | Coverage |
|---|---|
| `tests/test_cost_model.py` | Taker fee, slippage, bridge, funding accrual, round-trip aggregate |
| `tests/test_funding_cycle_lock.py` | Every public function × priority branches; locked-flip rejection; phase/seconds-to-end boundaries |
| `tests/test_forecast_scoring.py` | α-cascade tail-monitor under synthetic stable + shifted distributions |
| `tests/test_frontier.py` | Capacity ceiling, critical AUM, Bernstein leverage bounds |
| `tests/test_portfolio.py` | Cross-symbol allocation + mandate-floor routing |
| `tests/test_rigorous.py` | Closed-form break-even hold at mean; fixed-point iteration |
| `tests/test_stochastic.py` | OU MLE recovery on synthetic data; ADF on known-stationary + known-non-stationary series; CVaR drawdown; Hurst recovery |
| `tests/test_misc.py` | Misc cross-cutting invariants |

## Integration points

- **`bot-rs/crates/bot-tests`** — loads `rust_fixtures/*.json` to verify the Rust ports match this Python reference bit-for-bit. Resolved via `CARGO_MANIFEST_DIR → crates/bot-tests → bot-rs → dol-public → strategy/rust_fixtures`.
- **`bot-rs/crates/bot-strategy-v3`** — Rust ports of `funding_cycle_lock.py`, `stochastic.py`, `fsm_controller.py`. New-feature work lands in Python first; then ports.
- **`docs/integration-spec.md`** — the precise contract that any bot implementation must satisfy: signal JSON schema (§5.2), 11 invariants (§4), preflight v0 subset (§3.5). A `bot-rs`-compatibility check is that `bot-runtime::live_gate` lists a superset of §3.5.

## Boundaries

- **No on-chain transactions.** Signals only.
- **Read-only access to historical funding data.** The scripts never write to upstream datasets. Local outputs (backtests, `live_funding.sqlite`) live in `output/`.
- **All output is files.** No network side effects other than read-polling.

## Ship status

| Surface | Status |
|---|---|
| Iron-law enforcement (`funding_cycle_lock`) | Shipped + parity-pinned |
| Cost model | Shipped + unit-tested |
| Stochastic primitives (OU MLE, ADF, CVaR, Hurst) | Shipped + parity-pinned |
| Rust parity harness | 23 fixture modules, 100% coverage for `bot-math` primitives |
| FSM controller | Shipped in Python; Rust port shipped (`bot_strategy_v3::fsm_controller`) |
| Risk stack (4-layer) | Shipped in Python; Rust port partial (bot signal JSON flags `_stub: true`) |
| Forecast scoring (α-cascade) | Shipped in Python; Rust port deferred |
| Slippage calibration | Shipped in Python; live calibration harness partial |
| Pacifica live poller | Working against `/api/v1/funding_rate/aggregated` |
| Backtest harness | `backtest_v3_historical.py` runs against archived funding data |
