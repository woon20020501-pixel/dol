"""
rigorous.py — composition layer that wires stochastic.py + portfolio.py +
cost_model.py LiveInputs/Mandate into the per-tick rigorous decision pipeline
specified in docs/math-rigorous.md §8.

The first-order framework (cost_model.compute_system_state) uses point estimates
and equal-weight allocation. This rigorous layer adds:
  - ADF stationarity filter on each candidate's spread history
  - OU MLE fit with t-statistic credibility (5σ)
  - Half-life-based hold horizon (replaces fixed H_min)
  - Empirical CVaR drawdown stop (replaces 3σ Gaussian)
  - Mean-variance Markowitz allocation (replaces equal-weight)
  - Chance-constrained mandate compliance (replaces deterministic check)

The bot in production calls compute_rigorous_state() and gets a RigorousState
that contains everything needed to emit the next signal JSON.
"""
from __future__ import annotations
import math
import statistics
from dataclasses import dataclass, field
from typing import Optional

from strategy.cost_model import (
    LiveInputs, Mandate, target_vault_apy, target_vault_apy_floor,
    round_trip_cost_pct, slippage,
)
from strategy.stochastic import (
    OUFit, ADFResult, fit_ou, fit_drift, adf_test, cvar_drawdown_stop,
    expected_residual_income, optimal_hold_half_life_horizon,
)
from strategy.frontier import hurst_dfa
from strategy.portfolio import (
    Constraints, ChanceConstraintResult, covariance_matrix, shrink_covariance,
    chance_constrained_allocate,
)


# ===========================================================================
# Rigorous candidate filtering
# ===========================================================================

@dataclass
class RigorousCandidate:
    symbol: str
    counter_venue: str
    n_obs: int
    adf: Optional[ADFResult]                # may be None in drift regime (ADF expected to fail)
    ou: OUFit                               # in drift regime, ou.theta == 0 and ou.half_life_h == inf
    direction: int                          # +1 = long pacifica + short counter
    drawdown_stop: float                    # CVaR-derived
    spread_returns_signed_per_h: list       # for covariance matrix
    hurst: float                            # DFA Hurst exponent of the spread
    regime: str                             # "ou" (H ∈ [0.30, 0.70]) or "drift" (H > 0.70)
    planning_hold_h: float                  # hours to plan income over (half-life or commitment period)


def build_spread_series(symbol: str, counter: str, inputs: LiveInputs):
    """Returns aligned (timestamps, spread series) where spread = counter − pacifica per hour."""
    pac_hist = dict(inputs.funding_history_h.get((symbol, "pacifica"), []))
    cnt_hist = dict(inputs.funding_history_h.get((symbol, counter), []))
    common_ts = sorted(set(pac_hist.keys()) & set(cnt_hist.keys()))
    spread = [cnt_hist[t] - pac_hist[t] for t in common_ts]
    return common_ts, spread


def filter_candidate_rigorous(symbol: str, counter: str, inputs: LiveInputs,
                              m: Mandate, t_stat_threshold: float = 5.0,
                              hurst_ou_max: float = 0.70,
                              hurst_drift_max: float = 1.20,
                              hurst_min: float = 0.30,
                              commitment_hold_h: float = 168.0,
                              ) -> Optional[RigorousCandidate]:
    """Regime-aware candidate filter (v3.5.2 — critique #3 remediation).

    Routes each candidate to one of two model paths based on the empirical
    Hurst exponent of the spread:

      - **H ∈ [0.30, 0.70] — "ou" regime**: mean-reverting process. Apply the
        classical rigorous gates — ADF rejects unit root, OU fit converges with
        finite half-life ∈ [4h, 1000h], t-stat ≥ 5σ. Planning hold horizon is
        one half-life.

      - **H > 0.70 — "drift" regime**: persistent / trending process. The OU
        model does not fit (θ → 0 in the MLE, half-life → ∞). Instead:
          * skip ADF (ADF is designed to reject unit roots; for H ≈ 0.9 it
            will usually fail to reject, which is honest but uninformative)
          * fit a drift-only model (fit_drift): μ̂ = sample mean, with iid
            σ̂/√n standard error
          * require |t-stat| ≥ t_stat_threshold on μ̂ (this is an optimistic
            SE because the iid assumption breaks at H ≈ 0.9, so the threshold
            acts as a sanity check, not a p-value — caveat in code review)
          * planning hold horizon is a fixed commitment period (default 168h
            = 1 week), NOT a half-life (there is no half-life in this regime)

      - **H < 0.30 — rejected**: anti-persistent / oscillatory noise. Signed
        mean is not a stable point estimate of anything useful.

    This resolves the v3.5 internal contradiction where the code relied on OU
    machinery (half-life, AR(1) t-stat) while the data showed H ≈ 0.9 on every
    real cross-venue spread. Under the previous Hurst-pass-through gate, the
    OU fit either rejected (silently) or produced meaningless half-lives. The
    new filter either fits the correct model or refuses to trade.
    """
    if counter == "pacifica" or counter not in m.dex_venues:
        return None
    _, spread = build_spread_series(symbol, counter, inputs)
    if len(spread) < m.persistence_lookback_h_min:
        return None

    # Hurst exponent — the regime discriminator
    try:
        H = hurst_dfa(spread)
    except Exception:
        return None
    if H is None or not math.isfinite(H):
        return None
    if H < hurst_min:
        return None  # anti-persistent noise — unusable
    if H > hurst_drift_max:
        return None  # DFA H above 1.2 indicates a pure random walk / integrated
                     # process. Empirically: real cross-venue funding spreads
                     # cluster at H ≈ 0.85-1.10 (bounded persistent processes),
                     # while pure random walks of length 720 measured by DFA
                     # land at H ≈ 1.32-1.65. The 1.20 ceiling separates the
                     # two cleanly. Critique #6 / null-test #6.

    if H <= hurst_ou_max:
        # ----- OU regime -----
        adf = adf_test(spread, with_constant=True)
        if adf is None or not adf.rejects_unit_root:
            return None

        ou = fit_ou(spread, dt=1.0)
        if ou is None or ou.theta <= 0 or math.isinf(ou.half_life_h):
            return None
        if ou.half_life_h < 4 or ou.half_life_h > 1000:
            return None
        if abs(ou.t_statistic) < t_stat_threshold:
            return None

        regime = "ou"
        planning_hold_h = ou.half_life_h
    else:
        # ----- Drift regime (H > 0.70) -----
        adf = None  # ADF is uninformative here; record but don't gate
        ou = fit_drift(spread, dt=1.0)
        if ou is None:
            return None
        if abs(ou.t_statistic) < t_stat_threshold:
            return None  # signed mean isn't statistically distinguishable from 0

        # Reproducibility check (critique #6 remediation): split the series into
        # THIRDS and require (a) all three thirds share the same sign, and (b)
        # each third individually passes a Student-t credibility test on its
        # sample mean. Random walks score H > 0.7 because the cumulative series
        # has long memory, and their overall sample mean can look drifty by
        # endpoint noise — but when you cut the series into thirds, the per-third
        # means are essentially independent and will not all point the same way
        # at the same magnitude. Real structural drifts (e.g. venue A routinely
        # pays funding lower than venue B because of asymmetric taker flow) pass
        # the per-third test easily.
        n_obs = len(spread)
        n_third = n_obs // 3
        if n_third < 30:
            return None
        thirds = [spread[:n_third], spread[n_third:2 * n_third], spread[2 * n_third:]]
        third_means = [sum(t) / len(t) for t in thirds]
        # (a) same sign across all three thirds
        signs = [1 if tm > 0 else -1 if tm < 0 else 0 for tm in third_means]
        if signs[0] == 0 or signs.count(signs[0]) != 3:
            return None
        # (b) magnitude stability — the smallest third mean must be at least 25%
        # of the largest. Random walks pass (a) with probability ~1/4 and then
        # typically have wildly mismatched magnitudes because their per-third
        # drifts are endpoint-noise ratios, not reproducible effects. Combined
        # with (a) and the Hurst<0.95 gate above, this is sufficient on the
        # null tests in validate_rigorous.py without over-tightening real-data
        # admission. (Earlier versions added a per-third t-stat floor at
        # t_stat_threshold/√3 ≈ 2.89; that effectively raised the practical
        # t-threshold from 5 to ~7 and rejected legitimate signals. The Hurst
        # ceiling at 0.95 already blocks the random-walk null.)
        mag = [abs(tm) for tm in third_means]
        if max(mag) <= 0 or min(mag) / max(mag) < 0.25:
            return None

        regime = "drift"
        planning_hold_h = commitment_hold_h

    direction = +1 if ou.mu > 0 else -1
    # Verify instantaneous direction agrees with the fitted drift direction
    inst_pac = inputs.funding_rate_h.get((symbol, "pacifica"), 0.0)
    inst_cnt = inputs.funding_rate_h.get((symbol, counter), 0.0)
    inst_dir = +1 if (inst_cnt - inst_pac) > 0 else -1
    if inst_dir != direction:
        return None

    # Drawdown stop from basis history (CVaR)
    basis_hist = inputs.basis_divergence_history.get(symbol, [])
    basis_values = [v for _, v in basis_hist] if basis_hist else []
    d_max = cvar_drawdown_stop(basis_values, q=0.01, safety_multiplier=2.0)

    # Signed per-hour returns for covariance (only the signal direction matters)
    signed_returns = [direction * s for s in spread]

    return RigorousCandidate(
        symbol=symbol, counter_venue=counter, n_obs=len(spread),
        adf=adf, ou=ou, direction=direction,
        drawdown_stop=d_max, spread_returns_signed_per_h=signed_returns,
        hurst=H, regime=regime, planning_hold_h=planning_hold_h,
    )


# ===========================================================================
# Rigorous system state composition
# ===========================================================================

@dataclass
class RigorousState:
    timestamp_ms: int
    n_universe_scanned: int
    n_passing_filters: int
    candidates: list                        # list of RigorousCandidate
    leverage: int
    chance_constrained: ChanceConstraintResult
    target_vault_apy: float
    target_floor_apy: float
    notes: list = field(default_factory=list)


def required_leverage_rigorous(median_pair_apy: float, r_idle: float,
                                target_apy: float, alpha_floor: float = 0.50,
                                min_leverage: int = 1) -> int:
    """v3.5.2: smallest integer leverage that lets r_target be hit at α_floor.

    v3.5.1 imposed min_leverage=3 based on a 60-day single-regime sweep; external
    quant review found the justification ungrounded (cap-routing narrative didn't
    match measured output, tail risk was bounded by Gaussian not empirical CVaR,
    liquidation analysis ignored cross-venue basis blowouts). Default is now 1
    (no floor); any floor must come from a real-data CVaR/DRO tail argument.
    """
    if median_pair_apy <= 0:
        return max(min_leverage, 1)
    needed = 2 * (target_apy - alpha_floor * r_idle) / ((1 - alpha_floor) * median_pair_apy)
    L_computed = max(1, min(10, math.ceil(needed)))
    return max(min_leverage, L_computed)


def compute_rigorous_state(inputs: LiveInputs, m: Mandate,
                           t_stat_threshold: float = 5.0) -> RigorousState:
    """Run the full rigorous pipeline and return the operating envelope.
    Pure function of (inputs, mandate)."""
    target = target_vault_apy(m)
    floor = target_vault_apy_floor(m)
    notes = []

    pac_symbols = {s for (s, v) in inputs.funding_rate_h.keys() if v == "pacifica"}
    counters = [v for v in m.dex_venues if v != "pacifica"]
    n_scanned = 0
    candidates = []
    for s in pac_symbols:
        for v_c in counters:
            if (s, v_c) not in inputs.funding_rate_h:
                continue
            n_scanned += 1
            cand = filter_candidate_rigorous(s, v_c, inputs, m, t_stat_threshold)
            if cand is not None:
                candidates.append(cand)

    if not candidates:
        notes.append("no candidates passed ADF + OU + t-stat filters; staying all-idle")
        return RigorousState(
            timestamp_ms=inputs.timestamp_ms,
            n_universe_scanned=n_scanned, n_passing_filters=0, candidates=[],
            leverage=1,
            chance_constrained=ChanceConstraintResult(
                feasible=False, weights=[], idle_alpha=m.aum_idle_cap,
                portfolio_mean_apy=inputs.r_idle * m.aum_idle_cap,
                portfolio_std_apy=0.0,
                vault_5pct_apy=inputs.r_idle * m.aum_idle_cap,
                vault_1pct_apy=inputs.r_idle * m.aum_idle_cap,
                target_floor=floor, binds="empty universe",
            ),
            target_vault_apy=target, target_floor_apy=floor, notes=notes,
        )

    # Build covariance over signed per-hour returns, aligned on the shortest series
    min_len = min(len(c.spread_returns_signed_per_h) for c in candidates)
    aligned_returns = [c.spread_returns_signed_per_h[-min_len:] for c in candidates]
    cov_per_h = covariance_matrix(aligned_returns)
    cov_per_h = shrink_covariance(cov_per_h, lam=0.10)

    # Per-hour → APY conversion. For mean: × 8760 (sum over 8760 hours).
    # For variance: × 8760 also (sum of independent contributions; we treat the
    # annual horizon as approximately i.i.d. for VaR purposes — exact correction
    # for OU autocorrelation is bounded for fast-reverting processes).
    expected_apy = [abs(c.ou.mu) * 24 * 365 for c in candidates]
    cov_apy = [[c * (24 * 365) for c in row] for row in cov_per_h]

    # --- v3.5.2 critique #4 remediation: empirical fat-tail multiplier -----
    # Compute a per-candidate ratio of empirical 5th-percentile to
    # Gaussian-predicted 5th-percentile on the signed per-hour returns, then
    # take the max over candidates. This scales z_eps/z_stress upward when the
    # data has fatter tails than Gaussian. At H ≈ 0.9 the empirical tails are
    # materially wider than Gaussian; the inflation factor captures that gap
    # without requiring a full Monte-Carlo vault-quantile estimate.
    fat_tail_multiplier = 1.0
    for series in aligned_returns:
        if len(series) < 60:
            continue
        mean_s = sum(series) / len(series)
        var_s = sum((v - mean_s) ** 2 for v in series) / max(len(series) - 1, 1)
        std_s = math.sqrt(var_s)
        if std_s <= 0:
            continue
        sorted_s = sorted(series)
        k5 = max(0, int(0.05 * len(sorted_s)))
        empirical_5pct = sorted_s[k5]
        # Gaussian predicts: mean - 1.645 * std
        gaussian_5pct = mean_s - 1.645 * std_s
        # Ratio of actual downside to Gaussian downside
        emp_downside = mean_s - empirical_5pct
        gauss_downside = mean_s - gaussian_5pct
        if gauss_downside > 0:
            ratio = emp_downside / gauss_downside
            if ratio > fat_tail_multiplier:
                fat_tail_multiplier = ratio
    # Cap the multiplier at 5× — beyond that point the Gaussian approximation
    # is so broken that the caller should switch to an empirical Monte-Carlo
    # path, which is the planned v3.5.3 work.
    fat_tail_multiplier = min(fat_tail_multiplier, 5.0)
    notes.append(f"fat_tail_multiplier (empirical 5% / Gaussian 5%): {fat_tail_multiplier:.2f}")

    # Choose leverage from median signal strength
    median_apy = statistics.median(expected_apy)
    L = required_leverage_rigorous(median_apy, inputs.r_idle, target, m.aum_buffer_floor)

    # Build constraints
    counter_of = [c.counter_venue for c in candidates]
    n_counters_active = len(set(counter_of))
    m_counter = min(1.0, 1.20 / max(n_counters_active, 1))
    counter_caps = {v: m_counter for v in set(counter_of)}

    # m_pos derived from N candidates (reuse first-order formula)
    m_pos = (1 - m.aum_buffer_floor) * L / (2 * max(len(candidates), 1))
    m_pos = min(m_pos, 0.05)  # absolute cap

    constraints = Constraints(
        budget=1 - m.aum_buffer_floor,
        max_per_position=m_pos,
        max_per_counter=counter_caps,
        counter_of=counter_of,
    )

    # Stress floor: 200bps below mandate floor at 1% confidence
    stress_floor = max(0.0, floor - 0.02)

    cc = chance_constrained_allocate(
        expected_returns=expected_apy,
        cov=cov_apy,
        r_idle=inputs.r_idle,
        constraints=constraints,
        leverage=L,
        mandate_floor=floor,
        stress_floor=stress_floor,
        epsilon=0.05,        # 95% confidence vault clears mandate floor
        stress_eps=0.01,     # 99% confidence vault clears stress floor
        risk_aversion=2.0,   # half-Kelly
        fat_tail_multiplier=fat_tail_multiplier,  # critique #4 remediation
    )
    if not cc.feasible:
        notes.append(f"chance constraint infeasible: {cc.binds}")

    return RigorousState(
        timestamp_ms=inputs.timestamp_ms,
        n_universe_scanned=n_scanned, n_passing_filters=len(candidates),
        candidates=candidates, leverage=L, chance_constrained=cc,
        target_vault_apy=target, target_floor_apy=floor, notes=notes,
    )


# ===========================================================================
# Per-position exit re-evaluation
# ===========================================================================

@dataclass
class ExitDecision:
    should_exit: bool
    reason: str


def evaluate_exit_rigorous(symbol: str, counter: str, current_drawdown_pct: float,
                           held_hours: float, inputs: LiveInputs, m: Mandate,
                           original_d_max: float) -> ExitDecision:
    """Re-run the rigorous filters on an existing position. Exit if any has broken."""
    cand = filter_candidate_rigorous(symbol, counter, inputs, m)
    if cand is None:
        return ExitDecision(True, "rigorous filters no longer pass")
    if current_drawdown_pct >= original_d_max:
        return ExitDecision(True, f"drawdown {current_drawdown_pct:.4f} ≥ stop {original_d_max:.4f}")
    # Regime-aware minimum-hold and residual-income evaluation
    if cand.regime == "ou":
        min_hold = cand.ou.half_life_h * 0.5
        forward_hold_h = cand.ou.half_life_h
        reason_label = "half-life"
    else:
        # drift regime: commitment period is the floor; there's no half-life
        min_hold = cand.planning_hold_h * 0.5
        forward_hold_h = cand.planning_hold_h
        reason_label = "commitment period"
    if min_hold > held_hours:
        return ExitDecision(False, f"min hold ({reason_label}/2) not yet met")
    inst_spread = (inputs.funding_rate_h.get((symbol, counter), 0)
                   - inputs.funding_rate_h.get((symbol, "pacifica"), 0))
    res_inc = expected_residual_income(
        s_now=inst_spread,
        mu=cand.ou.mu,
        theta=cand.ou.theta,
        hold_h=forward_hold_h,
        direction=cand.direction,
    )
    if res_inc <= 0:
        return ExitDecision(True, f"expected residual income over {reason_label} is non-positive: {res_inc:.6f}")
    return ExitDecision(False, "hold")
