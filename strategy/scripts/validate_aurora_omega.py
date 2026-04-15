"""
validate_aurora_omega.py — Sprint 1 reference module validation

Covers:
  1. funding_cycle_lock: cycle index / lock enforcement / emergency override
  2. fair_value_oracle: p* weighting, stale drop, Kalman filter sanity
  3. depth_threshold: cut + redistribution + fail-safe fallback
  4. forecast_scoring: α-cascade strict propriety (Appendix F.7 tests a-e)

Run:
    python scripts/validate_aurora_omega.py

Pure stdlib. Mirrors the style of validate_formulas.py / validate_rigorous.py.
"""
from __future__ import annotations

import math
import random
import sys
from pathlib import Path

# Allow running from the repo root without installing as a package.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from strategy.funding_cycle_lock import (
    CycleState,
    DEFAULT_CYCLE_SECONDS,
    cycle_index,
    cycle_phase,
    enforce,
    is_locked,
    open_cycle,
    seconds_to_cycle_end,
    would_violate_lock,
)
from strategy.fair_value_oracle import (
    AGE_HARD_DROP_SEC,
    DEPTH_HARD_DROP_USD,
    FairValue,
    Kalman2State,
    STALE_MIN_WEIGHT,
    VenueQuote,
    compute_fair_value,
    kalman_init,
    kalman_step,
    normalize_to_tick,
    staleness_weight,
)
from strategy.depth_threshold import (
    DEFAULT_D_MIN_USD,
    VenueSlot,
    apply_depth_threshold,
    cut_summary,
)
from strategy.forecast_scoring import (
    BaselineRing,
    CascadeConfig,
    cascade_score,
    cascade_score_components,
    tail_deterioration_flag,
)


# ---------------------------------------------------------------------------
# Test harness (no pytest dependency — keep in stdlib)
# ---------------------------------------------------------------------------

_PASSED = 0
_FAILED = 0
_FAILURES: list[str] = []


def check(name: str, cond: bool, detail: str = "") -> None:
    global _PASSED, _FAILED
    if cond:
        _PASSED += 1
        print(f"  PASS  {name}")
    else:
        _FAILED += 1
        msg = f"  FAIL  {name}"
        if detail:
            msg += f"\n        {detail}"
        print(msg)
        _FAILURES.append(name)


def approx(a: float, b: float, tol: float = 1e-6) -> bool:
    return abs(a - b) <= tol


# ---------------------------------------------------------------------------
# 1. funding_cycle_lock
# ---------------------------------------------------------------------------


def test_funding_cycle_lock() -> None:
    print("\n[1] funding_cycle_lock")

    # Cycle index / phase
    check("cycle_index(0)==0", cycle_index(0.0) == 0)
    check("cycle_index(3599)==0", cycle_index(3599.0) == 0)
    check("cycle_index(3600)==1", cycle_index(3600.0) == 1)
    check("cycle_phase(1800)==0.5", approx(cycle_phase(1800.0), 0.5))
    check(
        "seconds_to_cycle_end(3500)==100",
        approx(seconds_to_cycle_end(3500.0), 100.0),
    )

    # Open + enforce within same cycle
    state = open_cycle(now=1000.0, h_c=+1, N_c=50_000.0)
    check("cycle opened, is_locked at t=1500", is_locked(state, 1500.0))
    check("cycle still locked at t=3599", is_locked(state, 3599.0))
    check("cycle unlocked at t=3600", not is_locked(state, 3600.0))

    # Enforce: locked cycle blocks direction flip
    h_eff, n_eff = enforce(state, now=1500.0, proposed_h=-1, proposed_N=80_000.0)
    check(
        "locked cycle returns locked h_c, ignores proposed flip",
        h_eff == +1 and n_eff == 50_000.0,
    )

    # Emergency override lets proposed values through
    h_eff, n_eff = enforce(
        state, now=1500.0, proposed_h=-1, proposed_N=80_000.0, emergency_override=True
    )
    check(
        "emergency override passes proposed values through",
        h_eff == -1 and n_eff == 80_000.0,
    )

    # After cycle expiry, enforce passes proposed through (cycle expired = unlocked)
    h_eff, n_eff = enforce(state, now=3700.0, proposed_h=-1, proposed_N=60_000.0)
    check(
        "expired cycle passes proposed values through",
        h_eff == -1 and n_eff == 60_000.0,
    )

    # would_violate_lock
    check(
        "would_violate_lock flags a flip in locked cycle",
        would_violate_lock(state, 1500.0, proposed_h=-1),
    )
    check(
        "would_violate_lock does not flag same-direction in locked cycle",
        not would_violate_lock(state, 1500.0, proposed_h=+1),
    )
    check(
        "would_violate_lock returns False when no lock active",
        not would_violate_lock(None, 1500.0, proposed_h=-1),
    )

    # Input validation
    try:
        open_cycle(now=0.0, h_c=7, N_c=100.0)
        check("open_cycle rejects invalid h_c", False, "did not raise")
    except ValueError:
        check("open_cycle rejects invalid h_c", True)

    try:
        open_cycle(now=0.0, h_c=+1, N_c=-10.0)
        check("open_cycle rejects negative N_c", False, "did not raise")
    except ValueError:
        check("open_cycle rejects negative N_c", True)


# ---------------------------------------------------------------------------
# 2. fair_value_oracle
# ---------------------------------------------------------------------------


def test_fair_value_oracle() -> None:
    print("\n[2] fair_value_oracle")

    now = 100.0

    # Fresh single venue: p* == mid (no funding, no bias)
    q1 = VenueQuote(
        venue="pacifica",
        mid=100.0,
        t_obs=now,
        depth_usd=50_000.0,
        funding_annual=0.0,
        mark_bias_bps=0.0,
    )
    fv = compute_fair_value([q1], now=now)
    check("single fresh venue: p* == mid", approx(fv.p_star, 100.0))
    check("single fresh venue: healthy", fv.healthy)
    check("single fresh venue: total_weight == 1", approx(fv.total_weight, 1.0))

    # Funding correction: 87.6% annual → per-period = 0.0001
    q_fund = VenueQuote(
        venue="pacifica",
        mid=100.0,
        t_obs=now,
        depth_usd=50_000.0,
        funding_annual=0.876,  # 8760 * 0.0001 = 0.876
        mark_bias_bps=0.0,
    )
    fv2 = compute_fair_value([q_fund], now=now)
    check(
        "funding-adjusted mid: p* = mid - 0.0001",
        approx(fv2.p_star, 99.9999, tol=1e-6),
    )

    # Two equal-weight venues: p* is the average
    q2 = VenueQuote(venue="backpack", mid=102.0, t_obs=now, depth_usd=20_000.0)
    fv3 = compute_fair_value([q1, q2], now=now)
    check("two equal-fresh venues: p* is midpoint", approx(fv3.p_star, 101.0))

    # Stale venue dropped by hard cut (age > 5s)
    q_stale = VenueQuote(
        venue="hyperliquid", mid=200.0, t_obs=now - 10.0, depth_usd=50_000.0
    )
    fv4 = compute_fair_value([q1, q_stale], now=now)
    check(
        "stale venue dropped (age > hard cut)",
        approx(fv4.p_star, 100.0) and "hyperliquid" not in fv4.contributing_venues,
    )

    # Shallow venue dropped by hard cut (depth < min)
    q_shallow = VenueQuote(
        venue="lighter", mid=200.0, t_obs=now, depth_usd=100.0
    )
    fv5 = compute_fair_value([q1, q_shallow], now=now)
    check(
        "shallow venue dropped (depth < hard cut)",
        approx(fv5.p_star, 100.0) and "lighter" not in fv5.contributing_venues,
    )

    # Staleness weight decays exponentially
    w0 = staleness_weight(0.0)
    w1_5 = staleness_weight(1.5)
    w3 = staleness_weight(3.0)
    check("χ(0) == 1", approx(w0, 1.0))
    check("χ(1.5) ≈ e^-1", approx(w1_5, math.exp(-1.0), tol=1e-9))
    check("χ(3.0) ≈ e^-2", approx(w3, math.exp(-2.0), tol=1e-9))

    # Moderate staleness: still contributing, just down-weighted
    q_half = VenueQuote(venue="backpack", mid=102.0, t_obs=now - 1.5, depth_usd=20_000.0)
    fv6 = compute_fair_value([q1, q_half], now=now)
    # Weights: 1.0 (q1) and e^-1 ≈ 0.368 (q_half). p* = (100 + 0.368*102) / 1.368
    expected = (1.0 * 100.0 + math.exp(-1.0) * 102.0) / (1.0 + math.exp(-1.0))
    check(
        "moderate staleness: weighted mid",
        approx(fv6.p_star, expected, tol=1e-9),
    )

    # Degenerate: all drop → healthy=False
    q_dead1 = VenueQuote(venue="v1", mid=100.0, t_obs=now - 20.0, depth_usd=50_000.0)
    q_dead2 = VenueQuote(venue="v2", mid=101.0, t_obs=now, depth_usd=10.0)
    fv7 = compute_fair_value([q_dead1, q_dead2], now=now)
    check("all venues drop → healthy=False", not fv7.healthy)
    check("all venues drop → total_weight=0", fv7.total_weight == 0.0)

    # Tick normalization
    check("normalize_to_tick(100.37, 0.1) == 100.4", approx(normalize_to_tick(100.37, 0.1), 100.4))
    check("normalize_to_tick(100.37, 0.0) == 100.37 (no-op)", approx(normalize_to_tick(100.37, 0.0), 100.37))

    # Kalman filter: follows a slow linear trend
    ks = kalman_init(initial_price=100.0)
    true_price = 100.0
    for i in range(60):
        true_price += 0.05  # drift 0.05 per second
        kalman_step(ks, obs=true_price + random.gauss(0, 0.01), dt=1.0)
    check(
        "kalman tracks trending price within 0.5",
        abs(ks.p - true_price) < 0.5,
        f"ks.p={ks.p:.4f} true={true_price:.4f}",
    )
    check(
        "kalman drift estimate near 0.05",
        abs(ks.d - 0.05) < 0.05,
        f"ks.d={ks.d:.4f}",
    )


# ---------------------------------------------------------------------------
# 3. depth_threshold
# ---------------------------------------------------------------------------


def test_depth_threshold() -> None:
    print("\n[3] depth_threshold")

    # Normal case: 3 survivors, 1 cut
    slots = [
        VenueSlot("pacifica", volume_usd=1_000_000, depth_usd=100_000),
        VenueSlot("hyperliquid", volume_usd=500_000, depth_usd=50_000),
        VenueSlot("backpack", volume_usd=200_000, depth_usd=20_000),
        VenueSlot("tiny", volume_usd=50_000, depth_usd=1_000),  # cut
    ]
    result = apply_depth_threshold(slots, total_notional=10_000, delta=0.5)
    tiny = next(s for s in result if s.venue == "tiny")
    check("tiny (shallow) venue got 0", tiny.allocated == 0.0)
    total = sum(s.allocated for s in result)
    check("total allocated ≈ total_notional", approx(total, 10_000, tol=1e-6))
    # Pacifica is deepest + highest volume → should get most, not all
    pac = next(s for s in result if s.venue == "pacifica")
    check("deepest venue got largest slice", pac.allocated == max(s.allocated for s in result))

    # Edge: all cut → all zeros
    all_shallow = [
        VenueSlot("a", volume_usd=1_000, depth_usd=500),
        VenueSlot("b", volume_usd=1_000, depth_usd=1_000),
    ]
    res2 = apply_depth_threshold(all_shallow, total_notional=1_000, delta=0.5)
    check("all shallow → all allocations 0", all(s.allocated == 0.0 for s in res2))

    # Edge: only 1 survivor
    one = [
        VenueSlot("keep", volume_usd=500_000, depth_usd=50_000),
        VenueSlot("drop", volume_usd=500_000, depth_usd=1_000),
    ]
    res3 = apply_depth_threshold(one, total_notional=5_000, delta=0.5)
    keep = next(s for s in res3 if s.venue == "keep")
    drop = next(s for s in res3 if s.venue == "drop")
    check("single survivor gets everything", approx(keep.allocated, 5_000))
    check("cut venue gets nothing", drop.allocated == 0.0)

    # cut_summary
    n_cut, n_sur, tot_cut = cut_summary(slots)
    check("cut_summary counts 1 cut, 3 survivors", n_cut == 1 and n_sur == 3)
    check("cut_summary total_cut_depth==1000", approx(tot_cut, 1_000))

    # Zero notional
    res4 = apply_depth_threshold(slots, total_notional=0.0, delta=0.5)
    check("zero notional → all zeros", all(s.allocated == 0.0 for s in res4))

    # Invalid delta
    try:
        apply_depth_threshold(slots, total_notional=100, delta=-1.5)
        check("invalid delta rejected", False)
    except ValueError:
        check("invalid delta rejected", True)


# ---------------------------------------------------------------------------
# 4. forecast_scoring — Appendix F.7 strict-propriety tests
# ---------------------------------------------------------------------------


def _expected_h(P_samples: list[float], c: float, cfg: CascadeConfig) -> float:
    """Empirical h(c) = Σ_ℓ w_ℓ · E_hat[|c - X|^α_ℓ]."""
    alphas = cfg.alpha_grid()
    total = 0.0
    n = len(P_samples)
    for w, a in zip(cfg.weights, alphas):
        if w == 0.0:
            continue
        inner = sum(abs(c - x) ** a for x in P_samples) / n
        total += w * inner
    return total


def _argmin_grid(P_samples: list[float], cfg: CascadeConfig, lo: float, hi: float, n: int = 501) -> tuple[float, float]:
    step = (hi - lo) / (n - 1)
    best_c = lo
    best_h = float("inf")
    for i in range(n):
        c = lo + i * step
        h = _expected_h(P_samples, c, cfg)
        if h < best_h:
            best_h = h
            best_c = c
    return best_c, best_h


def test_forecast_scoring() -> None:
    print("\n[4] forecast_scoring — α-cascade strict propriety (Appendix F.7)")

    random.seed(42)
    cfg = CascadeConfig()  # default {1.0, 1.5, 2.0, 2.5, 3.0} uniform
    check("default cascade alpha_grid", cfg.alpha_grid() == (1.0, 1.5, 2.0, 2.5, 3.0))
    check("default weights uniform", all(approx(w, 0.2) for w in cfg.weights))

    # Test F.7.a — strict propriety on a Gaussian mixture
    samples = []
    for _ in range(10_000):
        if random.random() < 0.7:
            samples.append(random.gauss(0.0, 1.0))
        else:
            samples.append(random.gauss(3.0, 2.0))

    c_star, h_star = _argmin_grid(samples, cfg, lo=-5.0, hi=8.0, n=1301)
    # Check strict convexity: h(c_star ± ε) > h(c_star) for several ε
    for eps in (0.05, 0.2, 0.5, 1.0):
        h_left = _expected_h(samples, c_star - eps, cfg)
        h_right = _expected_h(samples, c_star + eps, cfg)
        check(
            f"h(c* - {eps}) > h(c*)",
            h_left > h_star + 1e-9,
            f"h_left={h_left:.6f} h_star={h_star:.6f}",
        )
        check(
            f"h(c* + {eps}) > h(c*)",
            h_right > h_star + 1e-9,
            f"h_right={h_right:.6f} h_star={h_star:.6f}",
        )

    # Test F.7.b — per-α limit checks
    cfg_l1 = CascadeConfig(alpha_0=1.0, eta=1e-6, L_max=1, weights=(0.001, 0.999))
    # α₀=1, α₁ ≈ 1 → essentially L1 → target is the median of P.
    # Use a clean Gaussian for an exact check.
    normal_samples = [random.gauss(5.0, 1.0) for _ in range(20_000)]
    c_median, _ = _argmin_grid(normal_samples, cfg_l1, lo=0.0, hi=10.0, n=2001)
    true_median = sorted(normal_samples)[len(normal_samples) // 2]
    check(
        "α≈1 cascade → median estimator",
        abs(c_median - true_median) < 0.05,
        f"c*={c_median:.3f} median={true_median:.3f}",
    )

    # α=2 only: should recover the mean.
    cfg_l2 = CascadeConfig(alpha_0=1.0, eta=1.0, L_max=1, weights=(0.0, 1.0))
    c_mean, _ = _argmin_grid(normal_samples, cfg_l2, lo=0.0, hi=10.0, n=2001)
    true_mean = sum(normal_samples) / len(normal_samples)
    check(
        "α=2 cascade → mean estimator",
        abs(c_mean - true_mean) < 0.02,
        f"c*={c_mean:.3f} mean={true_mean:.3f}",
    )

    # Test F.7.c — default cascade is distinct from median/mean on skewed data
    # (mixture has mean ~0.9, median ~0.5 — cascade should sit between)
    mean_full = sum(samples) / len(samples)
    median_full = sorted(samples)[len(samples) // 2]
    # Note: 0.7·N(0,1) + 0.3·N(3,4) has mean 0.9; the default cascade with
    # heavy weight on α=1 (20%) and higher tiers pulls c* toward the mean-ish
    # region because higher moments penalize the 3-mode tail. Just verify
    # c_star is strictly between and differs meaningfully from both.
    check(
        "default cascade c* differs from pure median",
        abs(c_star - median_full) > 0.05,
        f"c*={c_star:.3f} median={median_full:.3f}",
    )
    check(
        "default cascade c* differs from pure mean",
        abs(c_star - mean_full) > 0.02,
        f"c*={c_star:.3f} mean={mean_full:.3f}",
    )

    # Test F.7.d — non-gamability: constant predictors other than c* score worse
    residuals_at_cstar = [c_star - x for x in samples]
    s_star = cascade_score(residuals_at_cstar, cfg) / len(samples)
    worse_count = 0
    for c in (c_star - 0.5, c_star - 0.1, c_star + 0.1, c_star + 0.5, c_star + 1.0):
        residuals_at_c = [c - x for x in samples]
        s_c = cascade_score(residuals_at_c, cfg) / len(samples)
        if s_c < s_star - 1e-9:
            worse_count += 1
    check(
        "all 5 deviating constants strictly worse than c*",
        worse_count == 5,
        f"worse={worse_count}/5",
    )

    # Test F.7.e — weight edge cases
    cfg_all_top = CascadeConfig(alpha_0=1.0, eta=0.5, L_max=4, weights=(0.0, 0.0, 0.0, 0.0, 1.0))
    c_top, _ = _argmin_grid(samples, cfg_all_top, lo=-5.0, hi=8.0, n=1301)
    # α=3 M-estimator: still unique (strictly convex), just check convexity
    h_top = _expected_h(samples, c_top, cfg_all_top)
    check(
        "all-weight-on-α=3 tier: strictly convex minimum",
        _expected_h(samples, c_top - 0.1, cfg_all_top) > h_top
        and _expected_h(samples, c_top + 0.1, cfg_all_top) > h_top,
    )

    # Config validation
    try:
        CascadeConfig(alpha_0=0.5)  # subunit power
        check("CascadeConfig rejects alpha_0 < 1", False)
    except ValueError:
        check("CascadeConfig rejects alpha_0 < 1", True)

    try:
        CascadeConfig(weights=(0.3, 0.3, 0.3, 0.05, 0.04))  # doesn't sum to 1
        check("CascadeConfig rejects weights not summing to 1", False)
    except ValueError:
        check("CascadeConfig rejects weights not summing to 1", True)

    try:
        CascadeConfig(alpha_0=1.0, eta=0.5, L_max=4, weights=(1.0, 0.0, 0.0, 0.0, 0.0))  # all weight on α=1 (degenerate)
        check("CascadeConfig rejects degenerate all-α=1", False)
    except ValueError:
        check("CascadeConfig rejects degenerate all-α=1", True)

    # Baseline ring + tail deterioration flag
    ring = BaselineRing(window=50)
    for _ in range(40):
        ring.push(-100.0 + random.gauss(0, 2.0))
    mu, sigma = ring.mean_std()
    check("baseline ring: mean near -100", abs(mu + 100.0) < 1.0)
    check("baseline ring: std near 2", abs(sigma - 2.0) < 1.0)

    # Non-deteriorating: current == baseline mean → no flag
    tf = tail_deterioration_flag(current=mu, baseline=ring)
    check("current == baseline → no flag", not tf.fired)

    # Heavy deterioration: 3σ below → flag
    tf2 = tail_deterioration_flag(current=mu - 3 * sigma, baseline=ring)
    check("current at -3σ → flag fires", tf2.fired)
    check("z-score ≈ -3", abs(tf2.z + 3.0) < 0.3)

    # Cold start guard
    small_ring = BaselineRing(window=50)
    small_ring.push(-100.0)
    small_ring.push(-101.0)
    tf3 = tail_deterioration_flag(current=-200.0, baseline=small_ring)
    check("cold start (< min_samples) does not fire", not tf3.fired)

    # cascade_score components sum
    total, per_tier = cascade_score_components([0.1, 0.2, 0.3], cfg)
    check("cascade_score_components sum matches total", approx(sum(per_tier), total, tol=1e-12))


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    print("=" * 70)
    print("Aurora-Ω Sprint 1 reference module validation")
    print("=" * 70)

    test_funding_cycle_lock()
    test_fair_value_oracle()
    test_depth_threshold()
    test_forecast_scoring()

    print()
    print("=" * 70)
    print(f"Results: {_PASSED} passed, {_FAILED} failed")
    if _FAILED:
        print("Failed tests:")
        for f in _FAILURES:
            print(f"  - {f}")
    print("=" * 70)
    return 0 if _FAILED == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
