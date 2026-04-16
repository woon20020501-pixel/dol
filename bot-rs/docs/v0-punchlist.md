# v0 Live Promotion Punchlist

**Purpose:** track what separates the current demo build from a live-trading-capable v0 bot per `integration-spec.md` §3.5. Every item below must be cleared (or explicitly waived by PM in writing) before `RUNNER_ALLOW_LIVE=1` is set on any path that touches real orders.

**Snapshot tag:** `aurora-omega-1.1.3`
**Framework version:** v3.5.2 portfolio core + Aurora-Ω Sprint 1/2/3 modules (22 Python files, 210/210 tests)
**Bot version:** Week 1 Step B (demo build)
**Date written:** 2026-04-15
**Owner:** bot team

---

## Hard gate — currently blocking

Before any live submission may occur, ALL of the following must be TRUE:

- [ ] Every item in §1 "Invariant coverage" below has status **"OK for live"** or **"Waived by PM (ref: msg-NNN)"**
- [ ] Every item in §2 "v0 mandatory modules" below is implemented in Rust with parity tests green against Python reference
- [ ] The three bot-owned guards in §3 are in place and tested under fault injection
- [ ] Operator has set `RUNNER_ALLOW_LIVE=1` in the process environment (see `bot-runtime/src/live_gate.rs`)
- [ ] PM has signed off on live promotion

Current state: **BLOCKED.** The demo path is dry-run only by construction — `VenueAdapter::submit_dryrun` is the only path that exists. There is no live submission entry point yet, so `RUNNER_ALLOW_LIVE` is currently a defense-in-depth preflight for Week 2+ work.

---

## §1. Invariant coverage (integration-spec.md §4)

| ID | Wall | Status | Gap | Required for live |
|---|---|---|---|---|
| **I-LOCK** | §1 iron-law direction lock | 🟢 **Active** | — | `funding_cycle_lock::enforce` ported to Rust (`bot-strategy-v3::funding_cycle_lock`) and threaded through every decision via `cycle_lock::CycleLockRegistry`. Demo exercises this path. |
| **I-VENUE** | DEX-only whitelist | 🟢 **Active** | — | `bot_types::Venue` is a closed 4-variant enum; static check. No code path can construct a non-whitelisted venue. |
| **I-SAME** | Byte-identical symbol on both legs | 🟡 **Partial** | `PairDecision::symbol` is a single `String` — legs cannot drift. But pair construction (`decision::decide`) groups snapshots by symbol pre-construction. No construction-time equality assert exists. | Add explicit `debug_assert!(leg_a.symbol == leg_b.symbol)` in the order-construction path (when it comes online in Week 2+). |
| **I-KILL** | FSM emergency flatten in ≤ 120s | 🔴 **Missing** | `fsm_controller` not ported to Rust. No kill-switch wiring. | Port `fsm_controller` + wire `emergency_flatten` into runtime. Week 3+. |
| **I-FV** | No order when `fair_value.healthy == false` | 🟡 **Partial** | Demo's `fair_value.rs` is a simplified weighted mid — NOT the full `compute_fair_value` with Kalman / staleness weights / tier-based health. Healthy check is crude (`≥ 2 venues contributed`). `decision::decide` SHOULD gate on this but currently only requires ≥ 2 venues implicitly. | Port full `fair_value_oracle::compute_fair_value`; gate order submission on `healthy == false`. Week 2+. |
| **I-FCST** | `tail_deterioration_flag(fired=true)` fed into FSM same tick | 🔴 **Missing** | `forecast_scoring` not ported. Signal JSON emits stub zeros. | Port `forecast_scoring.cascade_score` + `BaselineRing` + `tail_deterioration_flag`. Warm-start BaselineRing from `dry_run_v3_5.py` (60+ entries). Week 3+. |
| **I-DEPTH** | Every allocated slot `depth ≥ d_min` or `allocated = 0` | 🔴 **Missing** | `depth_threshold::apply_depth_threshold` not ported. Demo does NOT apply a depth cut — weighted fair value uses raw depth top as a weight but doesn't reject shallow venues. | Port `depth_threshold` + `fractal_delta`. Gate order construction on allocation > 0. Week 3+. |
| **I-PARTIAL** | Hedge notional ≥ `dynamic_q_min` OR batched via `ResidualPool.net` | 🔴 **Missing** | `partial_fill_model` not ported. Demo doesn't distinguish hedge legs from maker posts. | Port `partial_fill_model` (BetaPosterior + ResidualPool + kaplan_meier_survival + dynamic_q_min). Week 3+. |
| **I-TOX** | `ToxicityDecision.cancel == true` → no post | 🔴 **Missing** | `toxicity_filter` not ported. Demo has no maker post path at all. | Port `toxicity_filter` with `LinearToxicityModel` default stub. Week 3+. |
| **I-IOC** | Every hedge leg passes `hedge_ioc::viable` | 🔴 **Missing** | `hedge_ioc` not ported. Demo doesn't submit anything. | Port `hedge_ioc` (p_ioc, viable, RetryState, failover_ranking). Week 3+. |
| **I-BUDGET** | BOTH `cvar_guard(losses, BUDGET_99)` AND `BUDGET_95` every tick | 🔴 **Missing** | `risk_stack` not ported. Demo does NOT compute CVaR or run guards. Aurora-Ω v0 spec says cvar_guard + BUDGET_99 is MANDATORY for first live. | Port `risk_stack::cvar_ru` + `cvar_guard` + `BudgetTable`. Wire both guards into the tick pipeline. Week 2+. **Any live promotion without this is an iron-law breach.** |
| **I-SLIP** | Every fill → one `SlippageObservation`; recalibration via `apply_recalibration` | 🔴 **Missing** | `slippage_calibration` not ported. Demo uses a static 10 bps round-trip cost stub. | Port `slippage_calibration`. Wire `SlippageObservation` emission on the (future) fill path. Week 3+. |
| **I-FSM** | `fsm_step` called with all 5 axes every tick; mode transitions logged | 🔴 **Missing** | `fsm_controller` not ported. Demo FSM is a stub `{mode: "kelly_safe", notional_scale: 1.0}` — constant. | Port `fsm_controller::step` + `self_correcting_update` + `empirical_lipschitz_estimate`. v0 accepts reduced 3-axis FSM (§3.5). Week 2+. |
| **I-CONTRACT** | Callers of `self_correcting_update` monitor `empirical_lipschitz_estimate` | 🔴 **Missing** | Depends on I-FSM port. | Wire Lipschitz telemetry + alert when `> 0.9 · Δ_max`. Part of I-FSM port. |

**Color key:**
- 🟢 Active = Rust implementation exists, parity-tested, wired into tick pipeline
- 🟡 Partial = simplified placeholder in demo, does not yet match framework contract
- 🔴 Missing = no Rust implementation; stubbed or absent from signal JSON

**Hard requirement:** every 🔴 must become 🟢 for the corresponding invariant to be live-safe. PM may explicitly waive a 🟡 or 🔴 for a specific rollout tier (e.g., "Tier 0 paper trading on $10 notional for 24h is acceptable with I-DEPTH partial"), but the waiver must cite the rollout tier and the reason.

---

## §2. v0 mandatory modules (integration-spec.md §3.5)

The Aurora-Ω v0 minimum viable subset is 9 framework modules plus 3 bot-owned guards. Current Rust status:

| # | Module | Source | Rust status | Blockers |
|---|---|---|---|---|
| 1 | `funding_cycle_lock` | `strategy/funding_cycle_lock.py` | 🟢 **Ported** (`bot-strategy-v3::funding_cycle_lock`) | — |
| 2 | `fair_value_oracle` | `strategy/fair_value_oracle.py` | 🟡 **Simplified** (`bot-runtime::fair_value` — weighted mid only) | Kalman filter, staleness weights, tier-based health, `normalize_to_tick` |
| 3 | `depth_threshold` | `strategy/depth_threshold.py` | 🔴 Not started | — |
| 4 | `depth_allocator` (within `depth_threshold`) | same file | 🔴 Not started | — |
| 5 | `hedge_ioc` | `strategy/hedge_ioc.py` | 🔴 Not started | — |
| 6 | `partial_fill_model` | `strategy/partial_fill_model.py` | 🔴 Not started | — |
| 7 | `fsm_controller` (reduced 3-axis for v0) | `strategy/fsm_controller.py` | 🔴 Not started | — |
| 8 | `forecast_scoring` (shadow mode OK for v0) | `strategy/forecast_scoring.py` | 🔴 Not started | Needs `BaselineRing` + cold-start warm from `dry_run_v3_5.py` |
| 9 | `risk_stack::cvar_ru` + `cvar_guard` (BUDGET_99) | `strategy/risk_stack.py` | 🔴 Not started | **Critical: second line of defense against tail loss.** |

**Currently ported (not in the v0 minimum but useful):**
- `stochastic::fit_ou` — OU MLE
- `stochastic::adf_test` — Augmented Dickey-Fuller
- `stochastic::cvar_drawdown_stop` — empirical CVaR
- `stochastic::expected_residual_income`
- `stochastic::fit_drift` — drift regime fit

These are Phase 2a deliverables and are available for the decision engine when the full pipeline is wired up. They are NOT required for v0 per §3.5, but they anchor the v3.5.2 portfolio core layer.

---

## §3. Bot-owned guards (integration-spec.md §3.5)

| # | Guard | Owner | Status | Required behavior |
|---|---|---|---|---|
| 1 | `kill_switch` | Bot (not framework) | 🔴 Not implemented | SIGTERM trap OR file-flag (e.g. `touch HALT`). Cancels every open maker, flattens every open position via IOC, exits cleanly within 1 second of invocation. |
| 2 | `heartbeat` | Bot | 🔴 Not implemented | Hedge-fill subsystem emits a heartbeat every second. If no heartbeat for 5s, runner enters cooldown and emits only cancel orders until heartbeat resumes. |
| 3 | `Pacifica API watchdog` | Bot | 🔴 Not implemented | If Pacifica public API RTT exceeds 3s for three consecutive pings, runner enters emergency flatten via `fsm_controller.step` with `cooldown_active=True` forced. |

---

## §4. Deferred items (explicitly acknowledged by framework)

Per `integration-spec.md` §3.5 "v0 modules DEFERRED to v1+", these may be omitted from first live deployment but must be added before staircase Tier 2 ($1,000):

- `toxicity_filter` — stub with `p_tox=0.0` (no cancel). Exposes the bot to adverse-selection losses; `adverse_loss_bound` must be monitored.
- `fractal_delta` — use constant `δ = 0.35`. Depth allocator works, loses ~10% efficiency.
- `latency_penalty` — constant impact-only cost. Dimensionally correct but ignores timing-risk.
- `funding_bandit` — pin first hedge venue by `(funding_gain − expected_slippage)` argmax without UCB exploration. Greedy.
- `risk_stack` full 4 layers — v0 only needs `cvar_ru` + `cvar_guard` + `execution_chi2_report`. `entropic_ce` and `ecv` can be `red_flag=false` stubs.
- `offset_controller` — constant offset at `d_base_bps`. No toxicity boost.

---

## §5. Calibration obligations (integration-spec.md §6)

Parameters that must be refit from real data BEFORE live capital:

- [ ] `SlippageObservation` → `recalibrate_impact_coefficient` (use the framework hook — don't roll custom; it has truncation filtering + drift guard + R² gate)
- [ ] `Kalman2State.q_p / q_d / r` — 24–48h venue mid observations
- [ ] `forecast_scoring` baseline ring — warm-start from `dry_run_v3_5.py` on 60-day historical data
- [ ] `DEFAULT_BUDGET_99` — current values are from LHS (Appendix C, 100K scenarios); Phase 1 real losses should refine
- [ ] `cost_model.SLIPPAGE_IMPACT_COEFFICIENT` (`= 0.0008` Phase 0 placeholder) — must be refit before any promotion

OK to defer past Tier 1:
- `toxicity_filter.beta` ridge refit (stub `LinearToxicityModel` is safe)
- `latency_penalty.sigma_price_per_sqrts` per-venue (default placeholder is conservative)
- `fsm_controller.DEFAULT_MAX_STEP` (`= 0.02`, reasonable starting guard)
- `funding_bandit` exploration constant (`= √2`, standard)

---

## §6. Demo → live promotion checklist

This is the operational sequence to walk when promoting the demo path to a live-submitting path. Every box must be ticked or explicitly waived.

1. [ ] All 🔴 invariants in §1 have become 🟢 (or waived)
2. [ ] All v0 mandatory modules in §2 are ported and parity-tested (or waived for a specific rollout tier)
3. [ ] The three bot-owned guards in §3 are in place and tested under fault injection
4. [ ] Calibration obligations in §5 are complete for the parameters listed as "must refit"
5. [ ] Integration-spec §10 cross-reference audit walked (12 rows of PRINCIPLES enforcement verification)
6. [ ] `scripts/dry_run_v3_5.py` warm-started `BaselineRing` persisted to `persistence_store`
7. [ ] Chaos tests green (fallback, latency outlier, oracle stale)
8. [ ] Pacifica testnet smoke: real API round-trip with `--ignored` test for 72h continuous
9. [ ] Operator has verified `RUNNER_ALLOW_LIVE=1` is set in the target environment
10. [ ] Staircase Tier 0 ($10 paper) for 24h — observe signal JSON, NAV curve, decision log
11. [ ] Staircase Tier 1 ($100 live) — PM explicit signoff required
12. [ ] Staircase Tier 2 ($1,000) — requires 48h Tier 1 observation + PM signoff
13. [ ] Subsequent tiers follow the spec §11.4 with 48-72h observation windows

---

## §7. What is currently green and proof-of-life

As of 2026-04-15 (Week 1 Step B completion):

- 253 tests passing workspace-wide, 0 failing, 17 ignored (live-only or Phase 2-gated).
- `bot-math`: 18 pure functions, 56 internal sanity + 4 parity sections green against Python fixtures.
- `bot-strategy-v3::stochastic`: 5 Phase 2a functions ported with Python parity.
- `bot-strategy-v3::funding_cycle_lock`: full port of Python module, 13 unit tests.
- `bot-adapters`: `VenueAdapter` trait + `PacificaReadOnlyAdapter` + `DryRunVenueAdapter`. Live Pacifica API smoke test verified against real BTC funding rate.
- `bot-runtime`: `TickEngine` threads every decision through `CycleLockRegistry::enforce_decision`. `NavTracker` implements one-time round-trip cost + pure funding accrual on hold. Signal JSON matches integration-spec §5.2 with `cycle_lock` populated from real enforce state.
- `bot-runtime::live_gate`: `RUNNER_ALLOW_LIVE=1` preflight gate. Blocks any future live-submission path absent explicit opt-in.
- `bot-cli demo`: end-to-end subcommand. Live Pacifica + 3 DryRun fixture stubs, signal emission, NAV JSONL log, SIGINT handling. Verified end-to-end against real Pacifica BTC funding rate.
- I-LOCK enforcement is active on the demo path: mid-cycle pair flips and rebalances are blocked and logged.

**The demo is safe to run and show.** It is NOT safe to promote to live without clearing the §6 checklist.
