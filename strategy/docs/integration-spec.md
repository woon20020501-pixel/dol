# Framework ↔ Bot Integration Spec

**Document type:** Operational contract between the strategy framework and any bot implementation (Python or Rust).
**Status:** Authoritative contract. Any bot built against this framework must comply with every clause.

---

## 0. Read order for a new bot implementer

1. `PRINCIPLES.md` — iron law §1, what we are NOT doing (§2), residual basis-blowout risk (§5.1)
2. `docs/aurora-omega-spec.md` — 33-section Aurora-Ω master spec
3. `docs/math-aurora-omega-appendix.md` — Appendix F (α-cascade strict propriety proof)
4. `docs/math-formulas.md`, `docs/math-rigorous.md`, `docs/math-frontier.md` — v3.5.2 portfolio-core math
5. **This document** — the contract

---

## 1. Scope and boundaries

### 1.1 What the framework provides

The framework owns and delivers these artifacts. Bot implementations are free to consume them but must not modify:

| Artifact | Location | Purpose |
|---|---|---|
| v3.5.2 portfolio core | `strategy/cost_model.py`, `stochastic.py`, `portfolio.py`, `rigorous.py`, `frontier.py` | "Which (symbol, venue-pair) at what size?" |
| Aurora-Ω Sprint 1 | `strategy/funding_cycle_lock.py`, `fair_value_oracle.py`, `depth_threshold.py`, `forecast_scoring.py` | Iron law gate, fair value, depth cut, forecast monitoring |
| Aurora-Ω Sprint 2+3 | `strategy/fractal_delta.py`, `latency_penalty.py`, `partial_fill_model.py`, `toxicity_filter.py`, `offset_controller.py`, `hedge_ioc.py`, `fallback_router.py`, `funding_bandit.py`, `risk_stack.py`, `fsm_controller.py` | Execution-layer math |
| Validation suites | `scripts/validate_aurora_omega.py` (69/69), `scripts/validate_sprint2_3.py` (90/90) | Parity fixture source of truth |
| Formal spec | `docs/aurora-omega-spec.md` | 33-section contract semantics |
| Proofs | `docs/math-aurora-omega-appendix.md` | Appendix F |
| Real-data dry run | `scripts/dry_run_v3_5.py` | v3.5.2 parity target |

### 1.2 What the bot owns

The framework does NOT produce and does NOT block on:

- Tick scheduler / main event loop
- Venue API adapters (Pacifica, Hyperliquid, Lighter, Backpack signing, websockets, reconnect)
- Position tracker / order manager / order-ID correlation
- Cross-leg fill correlation and atomic open orchestration
- Paper-trading runner, DRY_RUN switches, operator runbook
- Rust runtime: tokio tasks, channels, actor supervision
- Secrets handling, API key rotation
- Telemetry export (Prometheus / Grafana) — framework provides the values; bot ships them
- Live dashboard, alerting, on-call rotation
- Kill switch, flatten-everything signal path

The bot team composes the call graph themselves. This document specifies the contract each framework module obeys; the bot decides the order and the state management.

### 1.3 Hard boundary

If a bot bug causes user funds loss, the first question is "did the bot obey every invariant in §4 of this document?" If yes, the framework is responsible for a framework flaw. If no, the bot implementation owns the incident. Treat §4 as load-bearing.

---

## 2. Module catalog (normative)

For each module: **what it is, its spec section, its key function signatures, its invariants the bot must respect.**

All modules are pure Python stdlib, zero external dependencies. The bot may port them to Rust (`bot-strategy-v3` crate in `bot-rs/`); parity is required to 6 decimal places against `validate_*.py` outputs.

### 2.1 `funding_cycle_lock` — Iron Law Gate (spec §3.1)

```python
from strategy.funding_cycle_lock import (
    CycleState, open_cycle, enforce, is_locked,
    would_violate_lock, cycle_index, seconds_to_cycle_end,
    DEFAULT_CYCLE_SECONDS,  # 3600
)
```

**Key contract — THE IRON LAW §1 ENFORCEMENT POINT.**

- `enforce(state, now, proposed_h, proposed_N, emergency_override=False) -> (h_eff, n_eff)` — the **only** code path the bot is permitted to use to determine cycle direction and notional. Inside a locked cycle, the returned `(h_eff, n_eff)` is the locked value; the caller's proposed flip is silently ignored unless `emergency_override=True`.
- `emergency_override=True` is a privileged escape hatch. The bot must log every invocation at WARN or higher, the operator must sign off on the rationale, and every override must be reviewable in the audit log. Use is restricted to: (a) FSM-triggered emergency flatten, (b) basis-blowout detector, (c) manual operator kill switch.
- No other code path may mutate `CycleState`. The bot must treat `state.cycle` as owned-by-framework through enforce/open_cycle only.
- Cycle cadence defaults to 3600s. The bot may override per-venue (e.g. Hyperliquid 8h historical) by passing a venue-specific `cycle_seconds`, but must NOT hardcode a cadence anywhere outside this module.

**Invariant I-LOCK:** For every emitted order with a non-zero `side`, the bot must have just called `enforce(...)` and the order's `side` must equal the returned `h_eff`. Violating this is an iron law breach.

### 2.2 `fair_value_oracle` — Cross-Venue Price Unification (spec §15-§17)

```python
from strategy.fair_value_oracle import (
    VenueQuote, FairValue, Kalman2State,
    compute_fair_value,       # core p* computation
    staleness_weight,         # χ(age) = exp(-age/τ)
    normalize_to_tick,        # §16.1
    kalman_init, kalman_step, # §16.3 lead/lag tracker
    clock_shift_correct,      # §16.2
    STALE_MIN_WEIGHT,         # 0.1 — §17 bound
    AGE_HARD_DROP_SEC,        # 5s
    DEPTH_HARD_DROP_USD,      # 1000
)
```

**Contract.**

- `compute_fair_value(quotes, now)` returns `FairValue(p_star, total_weight, contributing_venues, healthy)`.
- **If `healthy == False`, the bot MUST halt new maker posts and hedge submissions** until the next successful call. Existing positions should be held unchanged (not flattened — halting is not the same as flattening).
- All order pricing decisions must be made against `p_star`. **Using a venue's raw mid for order placement is a contract violation** (spec §15.2). Tick-rounding via `normalize_to_tick(p_star, venue.tick_size)` happens at the venue boundary.
- The Kalman filter is OPTIONAL per-venue lead/lag tracking. Default `q_p, q_d, r` are conservative defaults; the bot must calibrate from the first 24-48h of real fills before trusting the drift estimate (see §6 Calibration).

**Invariant I-FV:** No order may be submitted when the most recent `compute_fair_value` result has `healthy=False`.

### 2.3 `forecast_scoring` — α-Cascade Tail Monitor (spec §20, Appendix F)

```python
from strategy.forecast_scoring import (
    CascadeConfig,          # alpha_0=1.0, eta=0.5, L_max=4, uniform weights
    cascade_score,          # S(x_hat, x) = -Σ_ℓ w_ℓ Σ_k |Δx_k|^α_ℓ
    cascade_score_components,
    BaselineRing,           # rolling window, default 60 ticks
    tail_deterioration_flag, # returns TailFlag(fired, delta, z)
)
```

**Contract.**

- The bot collects residuals from every internal predictor (fractal δ estimator, Kalman oracle lead/lag, toxicity model, Beta partial-fill posterior, OU funding model, and any v3.5.2 predictors) each tick, concatenates them, and calls `cascade_score`.
- The bot maintains a `BaselineRing`, pushes `S_t` **after** evaluating the flag (so `S_t` is compared against a window that does NOT include itself).
- `tail_deterioration_flag(S_t, baseline, theta_sigma=2.0)` returns a `TailFlag`. When `fired=True`, this is the 5th red-flag axis into `fsm_controller.step`.
- `CascadeConfig.__post_init__` enforces the Appendix F.4 theorem assumptions (`alpha_0 >= 1`, non-degenerate weights). **The bot must not bypass validation** by constructing a config with `object.__setattr__` or similar.
- Cold start: `BaselineRing.is_ready(min_samples=10)` must return True before the flag is allowed to fire. The bot is responsible for warming the ring from a historical dry-run before enabling live order dispatch.

**Invariant I-FCST:** No `tail_deterioration_flag(fired=True)` result may be ignored by the FSM input on the tick it fires.

### 2.4 `depth_threshold` — Shallow Venue Cut (spec §5)

```python
from strategy.depth_threshold import (
    VenueSlot, apply_depth_threshold, cut_summary,
    DEFAULT_D_MIN_USD,  # 5000
)
```

**Contract.**

- `apply_depth_threshold(slots, total_notional, delta, d_min)` — hard cut + depth-aware redistribution on survivors.
- `delta` comes from `fractal_delta` live OLS; see §2.5. The bot must not hardcode `delta = 0.35`.
- If all venues are cut (`all(s.allocated == 0.0)`), the bot must halt new orders for this symbol (universe too shallow). Existing positions held.
- The bot may NOT widen the cut below `DEFAULT_D_MIN_USD = 5000` without policy sign-off. Tightening above is allowed during volatility spikes.

**Invariant I-DEPTH:** Every allocated slot returned by `apply_depth_threshold` has `depth_usd >= d_min` or `allocated == 0.0`.

### 2.5 `fractal_delta` — Live Impact Exponent (spec §6)

```python
from strategy.fractal_delta import (
    FractalFit, estimate_fractal_delta, delta_or_fallback,
    FALLBACK_DELTA,  # 0.35
    MIN_POINTS, MIN_R2,
)
```

**Contract.**

- Each tick the bot samples venue depth at 5+ price offsets to get a log-log depth curve, then calls `estimate_fractal_delta(delta_p, depth)`.
- `delta_or_fallback(fit)` returns the fitted δ if `fit.trusted`, else `FALLBACK_DELTA`. Use this value in `depth_threshold`, `latency_penalty`, `portfolio.chance_constrained_allocate`.
- The Aurora-Ω §6.3 proposition guarantees consistency of the OLS slope under standard assumptions; the proof is in `aurora-omega-spec.md` §6.3. Do not claim almost-sure convergence.

### 2.6 `latency_penalty` — MFG Execution Cost (spec §18)

```python
from strategy.latency_penalty import (
    VenueCostInputs, VenueCostBreakdown,
    latency_cost, impact_cost, total_venue_cost, breakdown,
)
```

**Contract.**

- Dimensional formula: `C^lat = α · τ · σ_flow · q² / D` in USD. The bot must pass τ in **seconds** and σ_flow in **s⁻¹**. Any unit confusion is a silent bug — the framework has no way to detect it at runtime.
- `total_venue_cost(inp, delta)` sums impact + latency and is what the bot should feed into the income-vs-cost decision gate (spec §4 of PRINCIPLES.md).
- σ_flow is a venue-specific calibrated constant (§6 calibration). Conservative defaults are provided; the bot re-calibrates from live data before trusting absolute values.

### 2.7 `partial_fill_model` — Beta(2,5) + Kaplan + FIFO (spec §10-§11)

```python
from strategy.partial_fill_model import (
    BetaPosterior,          # conjugate; initial Beta(2,5)
    ResidualPool,           # FIFO netting within a cycle
    SurvivalObs, kaplan_meier_survival, should_flatten_residual,
    dynamic_q_min,          # max(500, 20 * tickValue)
    size_hedge, HedgeDecision,
    PRIOR_A, PRIOR_B, SURVIVAL_FLATTEN_THRESHOLD,
)
```

**Contract.**

- Hedge sizing: `q_h = φ · q_m` where φ is the **observed** filled fraction. Use `use_expected_phi=False` for the canonical path. `use_expected_phi=True` is only for pre-fill scheduling (e.g. reserving latency budget).
- Minimum hedge threshold: `dynamic_q_min(tick_value_usd)`. If below, defer into `ResidualPool.add(direction, notional)` and batch; never emit an under-q_min hedge directly.
- `ResidualPool.net()` clears the pool and returns the single netted direction + notional. Call this at the end of a funding cycle or when the pool exceeds a bot-chosen size (recommended: on every Kaplan `should_flatten_residual` trigger).
- Beta posterior is persistent cross-cycle, but decays via `decay(factor)` toward the prior after regime-change detection. Recommended: decay with factor=0.5 when `fsm_controller` enters Robust mode.

**Invariant I-PARTIAL:** Every hedge leg the bot emits has `notional_usd >= dynamic_q_min(tick_value)` OR is a net-of-residual batch from `ResidualPool.net()`.

### 2.8 `toxicity_filter` — Maker Fill Toxicity (spec §7, §9)

```python
from strategy.toxicity_filter import (
    ToxFeatures, ToxicityModel, LinearToxicityModel,  # default stub
    ToxicityDecision, evaluate,
    adverse_loss_bound,  # §8: |PnL_adv| ≤ φ·Q·r*·Δt
    AucTracker, LabeledObs, ridge_refit_beta,
    DEFAULT_BETA, CANCEL_P_TOX, R_STAR,
)
```

**Contract.**

- `LinearToxicityModel` is the default stub (score via the linear formula, probability via logistic). The bot may replace with any `ToxicityModel` implementation (offline-trained GBT, online linear, etc.) as long as the Protocol is satisfied.
- `evaluate(features, model)` returns `ToxicityDecision(p_tox, score, cancel, offset_multiplier, features)`. If `cancel=True`, the bot MUST cancel / not-post this maker quote. No exceptions.
- `offset_multiplier` feeds `offset_controller` via the `(1 + λ p^0.7)` factor. Do not apply it twice.
- `AucTracker.needs_refit()`: when True, the bot is expected to collect ≥ 8 labeled observations and call `ridge_refit_beta`. Ridge refit failures (`None` return) are tolerated; the bot keeps the prior β.
- `adverse_loss_bound` gives an ex-ante upper bound. The bot should compare realized rolling adverse loss against it; exceeding the bound signals something OTHER than adverse selection (likely a data pipeline bug or misclassified toxic fill).

**Invariant I-TOX:** Every maker post the bot emits has either `tox.cancel == False` on the most recent evaluation, OR was emitted before the evaluation completed (scheduling race — must be bounded by the tick period).

### 2.9 `offset_controller` — Maker Quote Distance (spec §7.3)

```python
from strategy.offset_controller import (
    OffsetInputs, compute_offset_bps,
    DEFAULT_GAMMA, DEFAULT_LAMBDA, DEFAULT_D_MAX_BPS, DEFAULT_D_BASE_BPS,
)
```

**Contract.**

- `compute_offset_bps` returns bps, clipped to `[d_base, d_max]`. The bot must apply the returned offset to `p_star` to derive the limit price: `limit = p_star * (1 - side * offset_bps * 1e-4)`.
- `d_max` defaults to 20 bps and may NOT be widened without policy sign-off (§5 parameter lock).

### 2.10 `hedge_ioc` — IOC Certainty + Failover (spec §12-§13)

```python
from strategy.hedge_ioc import (
    p_ioc, viable,  # formula + MIN_P_IOC threshold
    FeeProfile, prefers_ioc,
    LatencyTracker,  # rolling RTT + z-score outlier
    RetryState,      # 80ms → 140ms → flatten
    VenueHedgeCandidate, failover_ranking, pick_primary,
    MIN_P_IOC, LATENCY_Z_CUT, RETRY_SCHEDULE_MS,
)
```

**Contract.**

- The bot calls `p_ioc(tau_ms, depth_usd)` per candidate venue and uses `failover_ranking(candidates)` to get a `D/τ`-ranked list of viable venues (those passing `viable(...)`).
- `RetryState.step()` returns the next backoff delay in ms, or `None` to signal emergency flatten. The bot must honor the `None` return by triggering `fsm_controller` emergency flatten via the framework, not by inventing its own fallback sequence.
- Latency outliers (`LatencyTracker.is_outlier`) must be skipped — don't submit during a latency spike.
- `prefers_ioc(fee_profile)` returns False when queue-join is cheaper; in that case the bot posts a maker instead of crossing.

**Invariant I-IOC:** No hedge leg may be submitted to a venue `c` where `viable(c.tau_ms, c.depth_usd) == False`.

### 2.11 `fallback_router` — Route + Cost Distribution (spec §14)

```python
from strategy.fallback_router import (
    Route, build_route,
    sample_fallback_spread_bps,   # Exp(λ) sampler
    expected_fallback_cost_usd,   # E[ξ] · q
    cvar_fallback_cost_usd,       # α-CVaR closed form
    FALLBACK_MEAN_BPS, FALLBACK_LAMBDA,
)
```

**Contract.**

- Build a route once per hedge decision via `build_route(failover_ranking(...))`. On each failure, call `route.next_fallback(failed_venues)` to get the next candidate.
- Cost estimation is analytic (closed-form for Exp distribution); the sampler is for simulation / stress testing only.

### 2.12 `funding_bandit` — UCB1 Venue Selection (spec §19)

```python
from strategy.funding_bandit import (
    BanditState, ArmStats, empirical_regret_fit,
    DEFAULT_EXPLORATION_C, DEFAULT_SLIPPAGE_WEIGHT,
)
```

**Contract.**

- `BanditState` is persistent cross-cycle state the bot owns. Initial UCB returns `inf` for unplayed arms, so first K ticks explore each arm once.
- Reward: `reward(funding_gain, expected_slippage) = funding_gain - λ_s · slippage`. The bot feeds in the **realized** funding after settlement and the **estimated** slippage at decision time.
- Regret: UCB1's theoretical bound is `O(sqrt(K T log T))`. The empirical fit helper `empirical_regret_fit(t) ≈ 0.19 sqrt(t)` is a TELEMETRY REFERENCE LINE ONLY. It is not a theorem. Do not cite it as one.
- When `fsm_controller` enters Robust, the bot should narrow the arm set (drop low-N arms) rather than reset. A bandit reset discards prior information the framework earned the hard way.

### 2.13 `risk_stack` — 4-Layer Risk Accounting + CVaR Budget Guard (spec §21, §28)

```python
from strategy.risk_stack import (
    RiskReport,
    entropic_ce, entropic_ce_report,
    ecv, ecv_report, sample_std,
    cvar_empirical, cvar_ru, cvar_report,
    chi_square, execution_chi2_report,
    # Budget guard (Fix 4, review response #4)
    BudgetTable, GuardAction, cvar_guard,
    DEFAULT_BUDGET_95, DEFAULT_BUDGET_99,
)
```

**Contract — 4-layer risk stack.**

- Each layer produces a `RiskReport(layer, value, threshold, red_flag, detail)`. The bot calls all four each tick on the rolling PnL loss window.
- Thresholds are mandate constants, NOT backtest-fit. Current defaults are conservative defaults and will be finalized from Phase 1 live data.
- `entropic_ce` uses `L = -log(1+R)` for Kelly linkage (spec §21.1). The bot must supply losses in this form to preserve the theoretical interpretation.
- `ECV = CVaR + κ · Std` — dimensionally consistent. Do NOT revert to `CVaR + κ · Var` (§29.2 deletion).

**Contract — CVaR budget guard (NEW, spec §28).**

- `cvar_guard(losses, table)` is **a second line of defense independent of the 4-layer red-flag counter**. The bot calls it each tick with:
  - `losses` — rolling portfolio loss window
  - `table` — a `BudgetTable` (use `DEFAULT_BUDGET_99` and `DEFAULT_BUDGET_95` for deep and fast guards respectively)
- The guard returns a `GuardAction` with `notional_scale` and `halt` flag.
- **Apply order:** the guard's `notional_scale` multiplies the FSM's `notional_scale`. If either the guard OR the FSM says halt, the bot halts. The two are AND-combined on the "continue" side and OR-combined on the "halt" side.
- Run BOTH `DEFAULT_BUDGET_99` (deep) and `DEFAULT_BUDGET_95` (fast) each tick. The fast guard reacts earlier; the deep guard catches tail events the fast guard misses.
- `BudgetTable` enforces `budget < warning < halt`. The bot must not construct non-monotone tables.
- The default tier values are **PROVISIONAL** from Appendix C LHS. Phase 1 live data must replace them before live capital.

**Invariant I-BUDGET:** On every tick, BOTH `cvar_guard(losses, DEFAULT_BUDGET_99)` AND `cvar_guard(losses, DEFAULT_BUDGET_95)` are called, and the bot's effective notional scale is the product of their `notional_scale` and the FSM's `notional_scale`. If any guard returns `halt=True`, the bot halts new orders.

### 2.13b `slippage_calibration` — v3.5.2 Phase 1 refit hook (cost_model addendum)

```python
from strategy.slippage_calibration import (
    SlippageObservation, SlippageCoefficients, RecalibrationReport,
    slippage_with_coefficients,
    recalibrate_impact_coefficient,
    apply_recalibration,
    MIN_RECAL_OBS,             # 30
    MAX_RECAL_CHANGE_FACTOR,   # 3.0
    MIN_RECAL_R_SQUARED,       # 0.20
)
```

**Contract.**

- The v3.5.2 `cost_model.SLIPPAGE_IMPACT_COEFFICIENT = 0.0008` is a Phase 0 conservative default. PRINCIPLES §2 requires no backtest-derived fixed values in the decision path; this constant is a narrowly-scoped exception pending Phase 1 recalibration. This module closes the loop.
- The bot emits a `SlippageObservation(notional_usd, oi_usd, vol_24h_usd, realized_slippage, ts)` for every fill, where `realized_slippage` is computed as `abs(fill_price - p_star) / p_star` using the oracle's `p_star` at submission time.
- The bot maintains a rolling window of observations (typical size: last 24h or last 500 fills, whichever is larger) and periodically calls `recalibrate_impact_coefficient(observations, current)`.
- The returned `RecalibrationReport` must be LOGGED IN FULL (not just the accept bit) for audit. The report captures why a refit was rejected and how far the fit diverged from prior.
- The bot applies the refit via `apply_recalibration(current, report)`, which returns an unchanged `current` if the report was rejected. **The bot MUST NOT bypass `apply_recalibration` by reading `report.new_impact_coefficient` directly** — doing so would skip the acceptance gate.
- Runtime slippage evaluation switches from `cost_model.slippage(Q, OI, VOL)` (module-constant path) to `slippage_with_coefficients(Q, OI, VOL, coef)` (runtime path) once a `SlippageCoefficients` exists. Both formulas are numerically identical when `coef == SlippageCoefficients.defaults()`.
- The bot persists `SlippageCoefficients` across runtime restarts via `persistence_store` (§6). On cold start without a stored copy, use `SlippageCoefficients.defaults()` and log a WARN.

**Filtering.** The recalibration function filters out observations that hit the floor (`≤ FLOOR`) or ceiling (`≥ CEILING`) of the clip, because those values are not the model's true output — they are truncation artifacts. Including them biases the fit toward the bounds.

**Acceptance checks.** The default thresholds are:
- `min_obs = 30` — sufficient for a stable OLS-through-origin estimate
- `max_change_factor = 3.0` — reject refits whose c_hat/c_old is outside [1/3, 3]; large jumps usually indicate a pipeline bug
- `min_r_squared = 0.20` — slippage is noisy, so 0.20 is lenient; the caller may tighten for production

**Invariant I-SLIP (new):** Every live fill emits exactly one `SlippageObservation`. The bot's rolling window is consulted every N ticks (operator-chosen, default hourly) to call `recalibrate_impact_coefficient`, and the coefficient change (if accepted) takes effect on the next tick. No code path may mutate `SlippageCoefficients` outside `apply_recalibration`.

**What this does NOT do.**
- Does not refit the two depth-fraction constants (`OI_FRACTION_AS_DEPTH`, `VOL_FRACTION_AS_DEPTH`). Those have cross-correlated effects with the impact coefficient and require a 2D nonlinear fit outside v1 scope.
- Does not refit the floor/ceiling. Those are mandate-set.
- Does not persist the coefficients — that's the bot's `persistence_store` responsibility.

---

### 2.14 `fsm_controller` — 5-Axis FSM + Self-Correcting Adapter (spec §22-§24)

```python
from strategy.fsm_controller import (
    Mode,                          # KELLY_SAFE / NEUTRAL / ROBUST
    FsmState, FsmDecision,
    step,                          # main entry
    self_correcting_update,        # clipped θ adapter
    empirical_lipschitz_estimate,  # Lemma S3 monitor
    DEFAULT_MAX_STEP,              # 0.02
    RED_FLAG_LIMIT,                # 2
    EMERGENCY_FLATTEN_SECONDS,     # 120
)
```

**Contract — FSM.**

- `step(state, now, reports, forecast_flag, funding_healthy, cooldown_active)` is the unique FSM entry point. The bot must pass ALL 4 risk reports AND the forecast flag. Passing a subset is a contract violation.
- `FsmDecision.notional_scale` is applied multiplicatively to every emitted order's notional. `FsmDecision.emergency_flatten=True` means the bot must (a) cancel every open maker, (b) flatten every open position via IOC, (c) set `cooldown_active=True` for the next `emergency_flatten_seconds` ticks.

**Contract — self-correcting adapter (Fix 3, review response #3).**

- `self_correcting_update(theta, realized_reward, utility, lam, beta, max_step=0.02)` computes the §24 map $\mathcal T$ with a hard-clip safeguard on the update step.
- **The hard-clip is NOT a proof of contraction.** The caller must not assume $L_{\mathcal T} < 1$ just because the clip is in place. The clip guarantees bounded motion; contraction is a separate property measured empirically via `empirical_lipschitz_estimate`.
- The bot MUST log both the unclipped raw $\mathcal T(\theta_t)$ and the clipped output, so the operator can compute the realized step magnitude against $\Delta_{\max}$ and decide whether the adapter is stable enough for production.
- `empirical_lipschitz_estimate(theta_history, t_history, window)` returns a conservative upper bound on realized step magnitude over a window. Expose this as telemetry; alert when it consistently exceeds $0.9 \cdot \Delta_{\max}$ (clip is binding).

**Invariant I-FSM:**
1. Every tick: FSM `step` is called with all 5 axes.
2. When `emergency_flatten=True`, no new orders are emitted and existing positions are flattened within 120 seconds.
3. Mode transitions are logged for audit.

**Invariant I-CONTRACT (new):** Callers of `self_correcting_update` must not rely on convergence without independently computing `empirical_lipschitz_estimate` and confirming it stays below $\Delta_{\max}$. Untracked adapter drift is a configuration error.

---

## 3. Tick call order (recommended, NON-NORMATIVE)

The bot team may compose modules in any order that preserves the invariants in §4. The following is a recommended reference sequence; treat it as a starting point, not a law.

```
for each tick:
    1. FETCH SNAPSHOTS       (bot-owned venue adapters)
    2. compute_fair_value    → halt if unhealthy
    3. estimate_fractal_delta → δ for this tick
    4. collect forecast residuals → cascade_score → tail_deterioration_flag
    5. v3.5.2 portfolio core: compute_system_state → filter_candidate_rigorous → chance_constrained_allocate → proposed (h, N)
    6. funding_cycle_lock.enforce → (h_eff, N_eff)
    7. apply_depth_threshold → allocated venues
    8. for each allocated venue:
         a. toxicity_filter.evaluate → cancel or compute p_tox
         b. offset_controller.compute_offset_bps
         c. hedge_ioc.failover_ranking → route
         d. partial_fill_model.dynamic_q_min → validate hedge size
         e. latency_penalty.total_venue_cost → income-vs-cost gate
    9. risk_stack (CE, ECV, CVaR, χ²) on rolling PnL losses
   10. fsm_controller.step with 5 axes → FsmDecision
   11. If emergency_flatten → cancel + flatten path
       Else → apply notional_scale to surviving orders
   12. Emit signal JSON (§5.2 format) + submit via venue adapters
```

---

## 3.5 v0 minimum viable fail-safe subset (review response #2)

The full framework has 14 modules. For the first live deployment, the bot team MAY ship a reduced v0 subset that still preserves all iron-law invariants and achieves "minimal but crash-tolerant" fail-safe status.

### v0 modules (mandatory for first live)

1. `funding_cycle_lock` — iron law §1 gate
2. `fair_value_oracle` — cross-venue price unification, halt on unhealthy
3. `depth_threshold` — shallow venue cut
4. `depth_allocator` (within `depth_threshold`) — hedge distribution
5. `hedge_ioc` — IOC certainty, retry backoff, failover ranking
6. `partial_fill_model` — Beta posterior, q_min, residual netting
7. `fsm_controller` — with a REDUCED axis set (see below)
8. **`forecast_scoring`** — MUST ship in shadow mode (scores computed, flag evaluated, but does not contribute to FSM red-flag count). Required because without it there is no early-warning signal for predictor drift; shadow mode lets the bot collect baseline without affecting decisions.
9. **`risk_stack.cvar_ru` + `cvar_guard`** — MUST ship the deep guard (`DEFAULT_BUDGET_99`) even if other risk layers are deferred. This is the second line of defense against tail loss.

### v0 MUST also include these bot-owned guards (not in `strategy/`)

1. **kill_switch (bot-owned):** a single operator command (SIGTERM with trap, or a file-based flag like `HALT` touching a known path) cancels every open maker, flattens every open position via IOC, and exits the runtime cleanly. Must fire within 1 second of invocation.

2. **heartbeat (bot-owned):** the hedge-fill subsystem must emit a heartbeat every second. If no heartbeat for 5 seconds, the runner must enter cooldown and emit only cancel orders until heartbeat resumes. The framework does not implement this — the bot must.

3. **Pacifica API watchdog (bot-owned):** if Pacifica public API round-trip latency exceeds 3 seconds for three consecutive pings, the runner must enter emergency_flatten via `fsm_controller.step` with `cooldown_active=True` forced. This is the runtime equivalent of wall 4 (§4 I-KILL).

### v0 modules DEFERRED to v1+

The following may be omitted from v0 but must be added before the notional exceeds the staircase Tier 2 ($1,000):

- `toxicity_filter` — stub with a constant $p_{\text{tox}}=0.0$ in v0 (no cancel behavior). This means v0 is exposed to adverse-selection losses; the `adverse_loss_bound` must be monitored tightly.
- `fractal_delta` — use constant $\delta = 0.35$ in v0. Depth allocator will work; live calibration loses ~10% efficiency.
- `latency_penalty` — use constant impact-only cost. Dimensionally correct but ignores timing-risk and congestion components.
- `funding_bandit` — pin the first hedge venue by `(funding_gain - expected_slippage)` argmax without UCB exploration. Functionally equivalent to a greedy scheduler.
- `risk_stack` full 4 layers — v0 only needs `cvar_ru` + `cvar_guard` + `execution_chi2_report`; `entropic_ce` and `ecv` can be stubbed to `red_flag=False`.
- `offset_controller` — use constant offset at `d_base_bps`. No toxicity boost.

### v0 FSM reduced axes

The v0 FSM receives THREE axes instead of five:

1. `cvar_report` from `cvar_ru`
2. `execution_chi2_report`
3. `forecast_scoring` flag (shadow mode — still counted)

Plus the `cvar_guard` second line that operates independently. The RED_FLAG_LIMIT is TWO of the three axes, OR a `halt=True` from `cvar_guard`.

This is thin but crash-tolerant: the bot has tail-loss protection (cvar_guard), execution anomaly detection (χ²), and predictor-drift monitoring (forecast), plus the kill switch, heartbeat, and Pacifica watchdog from the bot-owned guards. Missing only the full CE/ECV risk stack and the toxicity/latency layers.

### v0 → v1 upgrade path

Moving from v0 to v1 requires:
1. Adding the deferred modules above, one at a time, with a 24-hour soak between each addition.
2. Regenerating parity fixtures against the expanded module set.
3. Running the chaos tests (fallback, latency outlier, oracle stale).
4. Passing the `I-BUDGET` and `I-CONTRACT` invariants (§2.13, §2.14) with non-stub implementations.
5. Operator signoff captured in `output/rollout/tier_N.md`.

---

## 4. Safety invariants (NORMATIVE — iron law preservation)

The bot MUST preserve every invariant below. These are the four walls around iron law §1.

### Wall 1 — Funding cycle lock

**I-LOCK:** Every order with non-zero `side` was produced from a call to `funding_cycle_lock.enforce(...)` in the same tick, and its side equals `h_eff`. Flipping direction without `emergency_override=True` is forbidden.

**Bot team audit:** grep every code path that produces an order. If the `side` is not traceable to an `enforce()` return value within the same tick, that code path is a wall 1 breach.

### Wall 2 — DEX-only venue whitelist

**I-VENUE:** The set of **top-level venues** reachable by the bot is exactly `{pacifica, hyperliquid, lighter, backpack}`. No code path may submit an order to any top-level venue outside this set. Adding a new top-level venue requires policy sign-off AND an update to this document AND an update to `PRINCIPLES.md §1`.

**Bot team audit:** the venue enum in the order manager should be a closed sum type with these four variants. Static check.

**HIP-3 sub-namespace clarification (aurora-omega-1.1.4):** Hyperliquid's HIP-3 framework allows permissionless perp DEX deployment on Hyperliquid infrastructure. Sub-namespaces are reached through the same HL API endpoint with distinct coin identifiers (e.g., `xyz:GOLD`, `xyz:SILVER` for trade.xyz perps). These sub-namespaces:

- **Do NOT require a new top-level `Venue` enum variant.** They are fetched through the existing `HyperliquidAdapter` with the appropriate coin_id.
- **Are admissible under the `hyperliquid` whitelist entry**, provided they meet the same non-custodial, non-KYC properties as HL itself (which is inherited by HIP-3 deployments by construction).
- **Must still pass I-SAME (wall 3)** — the hedge pair's symbol-to-coin-id mapping must resolve to a semantically identical underlying (e.g., Pacifica `XAU` perp ↔ trade.xyz `xyz:GOLD` perp, both referencing gold as underlying).

New HIP-3 sub-namespaces discovered post-2026-04-15 do not require a new policy decision provided they stay within the HL infrastructure (same API endpoint, same non-KYC guarantee). A new NON-HL venue (e.g., a hypothetical new DEX on a separate chain) still requires full policy sign-off and an amendment to both this document and `PRINCIPLES.md`.

**Known admissible HIP-3 sub-namespaces (2026-04-15)**:
- `xyz:GOLD` (trade.xyz) — hedge counterpart for XAU
- `xyz:SILVER` (trade.xyz) — hedge counterpart for XAG

### Wall 3 — Same-asset enforcement

**I-SAME:** For any hedge pair `(leg_A, leg_B)`, the `symbol` field is byte-identical on both legs. Near-matches (e.g. `cbBTC` ↔ `WBTC`) are NOT same-asset and are forbidden (`PRINCIPLES.md §1`, note on "deterministically convertible").

**Bot team audit:** pair construction must validate symbol equality at construction time.

### Wall 4 — FSM kill switch

**I-KILL:** When `fsm_controller.step` returns `emergency_flatten=True`:
1. Cancel every open maker within 1 tick.
2. Flatten every position via IOC within `emergency_flatten_seconds` (default 120).
3. Set `cooldown_active=True` on subsequent `step` calls until the cooldown expires.
4. During cooldown, no new orders are emitted — even if the next tick's reports are all green (cooldown wins).

**Bot team audit:** there must be exactly one kill-switch implementation; every order submission must gate on `cooldown_active == False`.

### Additional invariants

- **I-FV** (fair value oracle halt) — §2.2
- **I-FCST** (forecast flag propagation) — §2.3
- **I-DEPTH** (depth cut honored) — §2.4
- **I-PARTIAL** (q_min enforcement) — §2.7
- **I-TOX** (cancel honored) — §2.8
- **I-IOC** (viability required) — §2.10
- **I-BUDGET** (both CVaR budgets evaluated every tick) — §2.13
- **I-SLIP** (slippage recalibration goes through `apply_recalibration`) — §2.13b
- **I-FSM** (5 axes, flatten, audit) — §2.14
- **I-CONTRACT** (empirical Lipschitz monitored for self-correcting adapter) — §2.14

---

## 5. Data contracts

### 5.1 Framework input contract (what the bot provides to framework modules)

Every framework call requires the bot to supply inputs from LIVE market data. The framework does NOT read from files, databases, or the network; every input is passed explicitly by the bot.

| Framework module | Bot must supply | Update cadence |
|---|---|---|
| `compute_fair_value` | `VenueQuote` per venue (mid, t_obs, depth, funding_annual, mark_bias_bps, tick_size) | Every tick |
| `estimate_fractal_delta` | `(delta_p, depth)` log-log curve, ≥ 5 points | Every tick (or per funding cycle) |
| `cascade_score` | Residuals from every active predictor | Every tick |
| v3.5.2 `compute_system_state` | `LiveInputs` (AUM, funding history, OI, vol, fees, bridge, vault returns) | Every tick |
| `apply_depth_threshold` | `VenueSlot(volume_usd, depth_usd)` list | Every tick |
| `toxicity_filter.evaluate` | `ToxFeatures(r_sigma, sweep, obi, lead_lag, queue_pos)` from order book snapshot | Every quote decision |
| `offset_controller.compute_offset_bps` | Normalized mid vol (σ_m) and `p_tox` | Every quote decision |
| `hedge_ioc.p_ioc` | Measured `tau_ms` (RTT) and `depth_usd` | Every hedge decision |
| `funding_bandit.observe` | Realized funding gain and estimated slippage | Every settlement cycle |
| `partial_fill_model.BetaPosterior.update` | Observed (n_success, n_fail) from fills | Every fill event |
| `risk_stack.*_report` | Rolling PnL loss list | Every tick |
| `fsm_controller.step` | All 4 reports + forecast flag + `funding_healthy` | Every tick |

**Note:** "every tick" is bot-defined. The framework does not require a specific cadence. A reasonable starting cadence is 1 Hz with 5-minute settlement aggregation; the bot team owns this decision.

### 5.2 Framework output contract (signal schema)

When the bot emits a rebalance signal (e.g. for operator audit or for the dashboard), the JSON must include these fields. Additional bot-specific fields are permitted as `extra`.

```json
{
  "version": "aurora-omega-1.0",
  "ts_unix": 1776225863.123,
  "symbol": "BTC",

  "portfolio_core": {
    "customer_apy_estimate": 0.0800,
    "buffer_apy_estimate": 0.0478,
    "reserve_apy_estimate": 0.0142,
    "leverage": 3,
    "m_pos": 0.0136,
    "n_active_pairs": 46
  },

  "fair_value": {
    "p_star": 100.0037,
    "healthy": true,
    "total_weight": 3.72,
    "contributing_venues": ["pacifica", "hyperliquid", "lighter", "backpack"]
  },

  "cycle_lock": {
    "locked": true,
    "cycle_index": 100000,
    "h_c": 1,
    "N_c": 10000.0,
    "seconds_to_cycle_end": 3451.0,
    "emergency_override": false
  },

  "fractal_delta": {
    "zeta": 0.5,
    "delta": 0.333,
    "r_squared": 0.999,
    "trusted": true
  },

  "forecast_scoring": {
    "S_t": -12.34,
    "baseline_mean": -11.8,
    "baseline_std": 0.5,
    "z": -1.08,
    "flag_fired": false,
    "cascade_alpha_grid": [1.0, 1.5, 2.0, 2.5, 3.0]
  },

  "risk_stack": [
    {"layer": "entropic_ce",     "value": 0.003, "threshold": 0.02, "red_flag": false},
    {"layer": "ecv",             "value": 0.012, "threshold": 0.05, "red_flag": false},
    {"layer": "cvar",            "value": 0.010, "threshold": 0.05, "red_flag": false},
    {"layer": "execution_chi2",  "value": 5.0,   "threshold": 15.0, "red_flag": false}
  ],

  "fsm": {
    "mode": "kelly_safe",
    "red_flags_fired": [],
    "notional_scale": 1.0,
    "emergency_flatten": false,
    "cooldown_active": false,
    "rationale": "all green + funding healthy"
  },

  "orders": [
    {
      "venue": "pacifica",
      "symbol": "BTC",
      "kind": "maker_post",
      "side": 1,
      "notional_usd": 2855.01,
      "limit_price": 99.9788,
      "client_tag": "BTC:1776225863:pacifica:0001",
      "offset_bps": 2.1,
      "p_tox": 0.08,
      "reason": "delta=0.393 offset=2.1bps p_tox=0.08"
    }
  ],

  "single_venue_exposure": {
    "pacifica": 0.62,
    "hyperliquid": 0.18,
    "lighter": 0.09,
    "backpack": 0.11
  },

  "diagnostics": {
    "sigma_m": 0.002,
    "tick_interval_ms": 1000,
    "framework_commit": "<git-sha>",
    "bot_commit": "<git-sha>"
  }
}
```

**Required fields:** `version`, `ts_unix`, `symbol`, `fair_value.healthy`, `cycle_lock.*`, `forecast_scoring.{S_t, z, flag_fired}`, `risk_stack`, `fsm.{mode, notional_scale, emergency_flatten}`, `orders`.

**Write destination:** `output/signals/{symbol}/{yyyymmdd}/{ts_unix}.json`. One file per tick. Retention: minimum 30 days for audit, indefinite for rollout tiers.

### 5.3 Acknowledgment

The bot must write each signal to disk **before** dispatching the corresponding orders. If the signal write fails, the bot must halt dispatching for this tick and alert the operator. This ordering guarantees that every order sent to a venue has a matching on-disk audit record.

---

## 6. Calibration dependencies

These parameters are conservative defaults in the reference implementation and MUST be recalibrated from real data before live capital:

| Module | Parameter | Default (conservative) | Calibration source |
|---|---|---|---|
| `fair_value_oracle.Kalman2State` | `q_p` | 1e-4 | 24-48h live price observations |
| `fair_value_oracle.Kalman2State` | `q_d` | 1e-6 | 24-48h live price observations |
| `fair_value_oracle.Kalman2State` | `r` | 1e-3 | 24-48h venue mid noise |
| `latency_penalty` | `sigma_flow` | per-venue 0.01 s⁻¹ | Flow variance of realized fills |
| `latency_penalty` | `alpha` | 1.0 | Residual regression on executed orders |
| `latency_penalty` | `eta` (impact) | 0.01 | v3.5.2 slippage model fit |
| `toxicity_filter` | β vector | (0.6, 1.0, 0.5, 0.7) | Ridge refit on labeled fills (§9.3) |
| `toxicity_filter` | `t_max` | 1.0 | Offline GBT threshold from labeled data |
| `partial_fill_model.BetaPosterior` | `(a, b)` | (2, 5) | Beta posterior update on first 100 fills |
| `funding_bandit` | `lambda_s` (slippage weight) | 1.0 | Cost-vs-reward scale from Phase 1 data |
| `risk_stack` thresholds | CE / ECV / CVaR / χ² | conservative defaults | policy mandate + Phase 1 empirical distribution |
| v3.5.2 `cost_model` slippage coefficients | `SLIPPAGE_IMPACT_COEFFICIENT` etc. | hardcoded | Live fill regression (spec §5.2 critique #11) |

**Cold start procedure** (required before enabling live orders):

1. Run `scripts/dry_run_v3_5.py` against `data/historical_cross_venue.sqlite` to warm-start `BaselineRing` for `forecast_scoring` with 60+ entries. Serialize the warm state.
2. Load the warm state into the bot's `BaselineRing` before the first live tick.
3. Operate for 24-48h in paper mode with real venue reads (but `dry_run=True` on every adapter). Collect Kalman / toxicity / Beta / bandit data.
4. Refit the conservative defaults from the 24-48h window.
5. Operator signoff on the refitted values.
6. Only then enable live orders under the internal staircase rollout.

The bot team owns the persistence format for warm state. The framework does not specify it — use whatever makes parity tests easy.

---

## 7. Parity test obligations

### 7.1 Framework-side

- `scripts/validate_aurora_omega.py` — Sprint 1 modules + Appendix F.7. Currently 69/69.
- `scripts/validate_sprint2_3.py` — Sprint 2+3 modules. Currently 90/90.
- `scripts/validate_formulas.py` / `validate_rigorous.py` / `validate_frontier.py` — v3.5.2 portfolio core.
- `scripts/dry_run_v3_5.py` — v3.5.2 real-data dry run (customer 8.00% / buffer 4.78% / L=3).

Every test must pass on the revision tagged as the framework release. A release with any failing test is **not** a valid release.

### 7.2 Bot-side

The bot team must write parity tests that compare their bot's numerical outputs against the framework Python reference on canonical fixtures. Required parity targets:

1. **Module-level parity** — for every module in §2, a test that feeds the same inputs through the Python reference and the bot's implementation and asserts output equality to 6 decimal places.
2. **Pipeline-level parity** — given a fixture snapshot sequence, the bot's end-to-end output (signal JSON per §5.2) must match a canonical reference signal generated by a "reference bot" the bot team builds. The reference bot composes the modules in the §3 recommended order and serves as the arbiter of parity.
3. **Dry-run parity** — the bot's output on `data/historical_cross_venue.sqlite` must match `dry_run_v3_5.py` numbers: customer 8.00% / buffer 4.78% / reserve 1.42% / L=3 / 46 pairs.

### 7.3 Fixture versioning

Framework fixtures are tagged with the framework release. Parity tests specify which framework revision they target. Upgrading the framework requires regenerating fixtures and re-running bot parity tests.

---

## 8. Halt / degrade protocol

The bot must implement at least these halt/degrade states:

| Trigger | Bot action | Duration |
|---|---|---|
| `fair_value.healthy == False` | Halt new orders, hold positions | Until next healthy reading |
| `depth_threshold` all-cut | Halt new orders for this symbol | Until depth recovers |
| `forecast_scoring.flag_fired` (alone) | No halt; contributes 1 red flag to FSM | — |
| `risk_stack` 1 red flag | FSM → Neutral; notional × ~0.75 | Per tick |
| `risk_stack` 2+ red flags | FSM → Robust; `emergency_flatten=True`; cancel + flatten | `EMERGENCY_FLATTEN_SECONDS = 120` |
| `funding_cycle_lock` violation detected | Halt ALL orders; alert operator | Until manual review |
| Wall 1-4 breach detected | Halt ALL orders; alert operator; rollback to last known good config | Indefinite until policy signoff |
| Venue adapter disconnect | Pause orders to that venue; failover pending orders | Until reconnect |
| Kalman innovation spike (5σ) | Enter Robust; investigate (possible oracle manipulation) | Until manual review |
| Bandit arm monopoly (>90% on one arm for 1000+ pulls) | Alert; do NOT reset (spec §2.12) | Operator decision |
| Beta posterior runaway (`a + b > 1000`) | `decay(0.5)` toward prior; alert | Per-event |

---

## 9. Single-venue concentration alarm

Per `PRINCIPLES.md §1.5` post-revocation block: the current 46-pair universe has ~100% Pacifica concentration, which exceeds the `Mandate.max_single_venue_exposure = 0.60` design cap. This is a **known structural risk**, not a framework bug.

**Bot obligation:**
1. Compute single-venue aggregate exposure every tick (sum of both-leg notionals touching each venue, divided by total deployed).
2. When Pacifica exposure > 60%: log WARN, emit an operator alert, continue trading.
3. When Pacifica exposure > 90%: log ERROR, emit a stronger alert, continue trading but pause adding new Pacifica-anchored pairs.
4. When a venue's exposure fraction drops below 0.60 for the first time: log INFO, re-enable normal mode.

The concentration alarm does NOT participate in the FSM red-flag count. It is a separate monitoring axis because it reflects structural universe thinness, not a transient regime change.

---

## 10. Iron Law §1 and §2 cross-reference audit

The following concepts from `PRINCIPLES.md` are structurally enforced through specific framework modules. A bot team review before live trading should verify each enforcement point.

| PRINCIPLES clause | Enforcement module | Enforcement mechanism |
|---|---|---|
| §1 same-asset hedge | `venue_adapter` (bot-owned), pair construction | Symbol equality check at pair construction |
| §1 β=1.0 by construction | `funding_cycle_lock` | Direction is symbol-global within a cycle |
| §1 DEX-only | `venue_adapter` whitelist (bot-owned) | Static venue enum |
| §1 funding-only revenue | v3.5.2 `cost_model` income definition | `income > cost` gate uses funding, not directional P&L |
| §2.1 not directional bets | `funding_cycle_lock` + `cost_model` | Cannot flip direction mid-cycle; gate is funding-based |
| §2.2 not statistical pairs | v3.5.2 `rigorous.filter_candidate_rigorous` | Same-symbol cross-venue only, no β-hedge correlated pairs |
| §2.3 not cross-asset arb | v3.5.2 `cost_model.evaluate_trade_live` | Pair schema is `(symbol, venue_A, venue_B)` with symbol equality |
| §2.4 not single-venue | v3.5.2 `rigorous.filter_candidate_rigorous` | Both legs must be on different venues |
| §2.5 not HF rotation | `funding_cycle_lock` + `fsm_controller` | Cycle lock + bounded notional scale |
| §2.6 not yield-max over safety | `fsm_controller` + `risk_stack` | Red flags dominate notional |
| §2.7 not custodial | `venue_adapter` whitelist | DEX-only enum |
| §5.1 basis-blowout tail | `fsm_controller` emergency_flatten + `max_per_counter` cap | 5-axis kill switch + venue concentration cap |
| §1 same-asset (RWA addendum) | `diagnostics.oracle_divergence_risk` signal field + venue concentration cap | RWA hedge pairs (XAU/XAG/PAXG class) with independent oracles per leg are admissible under the iron law as an **annotated structural risk**, NOT a waiver. Every RWA pair signal sets `"structural"`; bot sizes these pairs no larger than crypto pairs and monitors oracle divergence as part of the basis-blowout envelope. See aurora-omega-spec §5.1 "RWA oracle divergence" addendum. |

The bot team should sign off on each row before live. Missing enforcement is a wall breach even if the test suite passes.

---

## 11. What this document does NOT cover

Explicitly out of scope for the framework. These are bot-side decisions:

- Tick cadence (1 Hz? 5 Hz? settlement-aligned?)
- Order correlation across ticks (maker fill → hedge IOC; the framework provides `partial_fill_model` but not the correlation state machine)
- Atomic cross-venue open orchestration (the framework assumes atomic, the bot must implement it)
- Warm-state persistence format (pickle? JSON? SQLite?)
- Observability stack (Prometheus? OpenTelemetry? custom?)
- Alerting routing (Slack? PagerDuty? email?)
- Secrets management
- Rust runtime details (actor model, channel types, tokio task supervision)
- Deployment (systemd unit, Docker, k8s)
- CI pipeline

Questions about framework semantics are welcome; the framework team will not implement any of the above.

---

## 12. Versioning

| Framework revision | Date | Notes |
|---|---|---|
| `aurora-omega-1.0` | Sprint 1 + Sprint 2+3 complete. 159/159 unit tests. §6 complexity freeze lifted. |
| `aurora-omega-1.1` | Review response applied. Latency split (impact/timing/congestion), fallback Exp+Pareto mixture, Beta(1,1) default prior option, CMDP reward for RL, hard-clip ↔ contraction separation (Lemma S3), 76k → 3-tier BudgetTable, Conditional Propositions A/B/C added. 174/174 unit tests. Appendix B, Appendix C added. New invariants I-BUDGET, I-CONTRACT. v0 minimum subset defined (§3.5). |
| `aurora-omega-1.1.1` | §18.2 `timing_risk_cost` module function shipped. v3.5.2 slippage calibration hook shipped as `strategy/slippage_calibration.py` with OLS-through-origin refit + truncation-bias filtering + drift guard + R² gate. New invariant I-SLIP. 210/210 unit tests. |
| `aurora-omega-1.1.2` | CVaR budget tiers tightened from 100,000-scenario LHS (50× the prior 2K bootstrap). CVaR_99 budget (p50) shifted −26% ($2,000 → $1,500); other tiers within ±12%. `DEFAULT_BUDGET_95/99` in `strategy/risk_stack.py` updated to LHS-derived round-number tiers. Appendix C §C.3 rewritten with raw quantiles from the 100K run. Spec §28.1 budget table updated. KL ↔ Entropic CE duality note added (Fenchel-Legendre / Donsker-Varadhan). 210/210 unit tests unchanged. |
| `aurora-omega-1.1.3` | Two minor additions: (a) §21.1-dual `η → ∞` ess-sup limit note (Donsker-Varadhan corollary), (b) Appendix D — OU Stationary Tail Rate Function with single-time derivation of $\Lambda^*(x) = \kappa(x-\theta)^2/\sigma^2$, analytical CVaR bound, and $\sigma^2/\kappa$ parameter coupling observation. 210/210 unit tests unchanged. |
| `aurora-omega-1.1.4` | **Universe expansion: 10-symbol demo + RWA hedge pairs.** Demo universe added XAU, XAG, PAXG alongside the 7 crypto symbols (BTC/ETH/SOL/BNB/ARB/AVAX/SUI). XAU and XAG use Pacifica ↔ trade.xyz (HIP-3 perp DEX on Hyperliquid infrastructure) via coin ids `xyz:GOLD` / `xyz:SILVER`. §4 Wall 2 I-VENUE clarified: HIP-3 sub-namespaces are admissible under the `hyperliquid` whitelist entry without requiring a new `Venue` enum variant. Known structural risk: RWA hedge pairs (XAU/XAG/PAXG) use independent oracles on each leg. This is NOT an iron-law waiver — it is an annotated tail risk sized by venue concentration caps, surfaced as `diagnostics.oracle_divergence_risk: "structural"` in signal JSON. 210/210 unit tests unchanged. |

Bumping the framework version requires:
1. All validation suites pass
2. `docs/aurora-omega-spec.md` + this doc updated
3. Git tag `framework/vX.Y.Z`
4. Parity fixtures regenerated under `tests/fixtures/vX.Y.Z/`
5. Bot implementations re-run parity tests before consuming the new revision

---

## 13. Contact

Wall breaches, parity failures, or suspected iron-law violations should be escalated immediately through the project's normal operator channels; they must not wait for an ordinary review cycle.

---

**End of integration spec.**
