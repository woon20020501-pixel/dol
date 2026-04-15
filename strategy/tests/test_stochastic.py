"""Tests for strategy.stochastic — OU MLE, ADF, drift, CVaR."""
import math
import random
import statistics

import pytest

from strategy.stochastic import (
    fit_ou,
    fit_drift,
    adf_test,
    expected_spread_at,
    expected_residual_income,
    optimal_hold_half_life_horizon,
    lower_tail_mean,
    upper_tail_mean,
    cvar_drawdown_stop,
    _generate_ou_sample,
    OUFit,
)


def test_fit_ou_returns_none_on_short_series():
    assert fit_ou([0.1] * 10) is None


def test_fit_ou_recovers_theta_on_strong_signal():
    sample = _generate_ou_sample(mu=5e-5, theta=0.10, sigma=1e-4, T=720, seed=1)
    fit = fit_ou(sample)
    assert fit is not None
    assert 0.05 < fit.theta < 0.20  # loose bracket around true 0.10
    # Half-life close to log(2)/0.10 ≈ 6.93 h
    assert 3.0 < fit.half_life_h < 15.0


def test_fit_ou_recovers_positive_mu_with_large_tstat():
    sample = _generate_ou_sample(mu=1e-4, theta=0.10, sigma=1e-4, T=720, seed=2)
    fit = fit_ou(sample)
    assert fit is not None
    assert fit.mu > 0
    assert abs(fit.t_statistic) >= 5.0


def test_fit_ou_degenerates_on_constant_series():
    # Constant series → sxx == 0 → returns None
    assert fit_ou([0.001] * 200) is None


def test_fit_ou_random_walk_produces_long_half_life_or_degenerate():
    rng = random.Random(11)
    rw = [0.0]
    for _ in range(719):
        rw.append(rw[-1] + rng.gauss(0, 1e-4))
    fit = fit_ou(rw)
    assert fit is not None
    # On a random walk, either b is out of stationary range (degenerate
    # branch, half_life = inf) or the fit finds a very slow mean reversion.
    # We require the half-life to be long (> 20h) OR infinite — either way,
    # not a punchy OU fit.
    assert math.isinf(fit.half_life_h) or fit.half_life_h > 20.0


def test_fit_drift_none_on_short_series():
    assert fit_drift([0.1] * 10) is None


def test_fit_drift_packs_theta_zero_half_life_inf():
    rng = random.Random(3)
    s = [rng.gauss(1e-4, 5e-5) for _ in range(200)]
    fit = fit_drift(s)
    assert fit is not None
    assert fit.theta == 0.0
    assert math.isinf(fit.half_life_h)
    assert fit.mu == pytest.approx(statistics.mean(s))


def test_fit_drift_tstat_is_mean_over_sem():
    rng = random.Random(4)
    s = [rng.gauss(1.0, 1.0) for _ in range(100)]
    fit = fit_drift(s)
    mean = statistics.mean(s)
    stdev = statistics.stdev(s)
    expected_t = mean / (stdev / math.sqrt(100))
    assert fit.t_statistic == pytest.approx(expected_t, rel=1e-6)


# ---------------------------------------------------------------------------
# ADF test
# ---------------------------------------------------------------------------

def test_adf_none_on_short_series():
    assert adf_test([0.1] * 20) is None


def test_adf_rejects_on_strong_mean_reverting():
    rejected = 0
    for trial in range(10):
        s = _generate_ou_sample(5e-5, 0.20, 1e-4, T=500, seed=trial)
        res = adf_test(s)
        if res and res.rejects_unit_root:
            rejected += 1
    assert rejected >= 6  # strong-power expectation


def test_adf_mostly_accepts_random_walk_null():
    rejected = 0
    for trial in range(20):
        rng = random.Random(100 + trial)
        rw = [0.0]
        for _ in range(499):
            rw.append(rw[-1] + rng.gauss(0, 1e-4))
        res = adf_test(rw)
        if res and res.rejects_unit_root:
            rejected += 1
    # Expected Type-I error ≈ 5% → ~1-4 rejections out of 20.
    assert rejected <= 6


# ---------------------------------------------------------------------------
# OU residual income closed-form
# ---------------------------------------------------------------------------

def test_expected_spread_at_decays_toward_mu():
    # s_t=2, mu=0, theta=0.1 → E[s_10] = 2 exp(-1.0) ≈ 0.7358
    val = expected_spread_at(s_now=2.0, mu=0.0, theta=0.1, hours_ahead=10.0)
    assert val == pytest.approx(2.0 * math.exp(-1.0))


def test_expected_spread_at_theta_zero_returns_s_now():
    assert expected_spread_at(1.23, 0.0, theta=0.0, hours_ahead=5.0) == 1.23


def test_expected_residual_income_drift_regime_linear_in_hold():
    # theta=0 → income = direction * mu * hold
    inc = expected_residual_income(s_now=0.5, mu=0.01, theta=0.0, hold_h=10.0, direction=1)
    assert inc == pytest.approx(0.10)
    inc_neg = expected_residual_income(s_now=0.5, mu=0.01, theta=0.0, hold_h=10.0, direction=-1)
    assert inc_neg == pytest.approx(-0.10)


def test_expected_residual_income_ou_closed_form():
    # Compare to closed-form: dir * (mu*T + (s0-mu)*(1-e^{-θT})/θ)
    s_now, mu, theta, T = 0.5, 0.1, 0.2, 5.0
    exact = 1 * (mu * T + (s_now - mu) * (1 - math.exp(-theta * T)) / theta)
    got = expected_residual_income(s_now, mu, theta, T, direction=1)
    assert got == pytest.approx(exact)


def test_optimal_hold_half_life_horizon_fallback_for_drift():
    drift_fit = OUFit(n_obs=100, a=0.0, b=1.0, sigma_eps=1.0,
                      mu=0.0, theta=0.0, sigma=1.0,
                      se_b=float("inf"), se_mu=1.0, se_theta=float("inf"),
                      half_life_h=float("inf"), t_statistic=3.0)
    assert optimal_hold_half_life_horizon(drift_fit) == 168.0


def test_optimal_hold_half_life_horizon_returns_half_life_in_ou():
    ou_fit = OUFit(n_obs=720, a=0.0, b=0.9, sigma_eps=0.01,
                   mu=0.0, theta=0.1, sigma=0.01,
                   se_b=0.01, se_mu=0.001, se_theta=0.01,
                   half_life_h=7.0, t_statistic=6.0)
    assert optimal_hold_half_life_horizon(ou_fit) == 7.0


# ---------------------------------------------------------------------------
# CVaR / tail means
# ---------------------------------------------------------------------------

def test_lower_tail_mean_empty():
    assert lower_tail_mean([], q=0.05) == 0.0


def test_upper_tail_mean_empty():
    assert upper_tail_mean([], q=0.05) == 0.0


def test_lower_tail_mean_picks_bottom_quantile():
    xs = list(range(100))
    # bottom 5% = [0..4] → mean = 2
    assert lower_tail_mean(xs, q=0.05) == pytest.approx(2.0)


def test_upper_tail_mean_picks_top_quantile():
    xs = list(range(100))
    # top 5% = [99, 98, 97, 96, 95] → mean = 97
    assert upper_tail_mean(xs, q=0.05) == pytest.approx(97.0)


def test_cvar_drawdown_stop_bootstrap_fallback():
    assert cvar_drawdown_stop([0.001, 0.002], q=0.01) == 0.005


def test_cvar_drawdown_stop_fat_tailed_larger_than_normal():
    rng = random.Random(0)
    normal = [rng.gauss(0, 0.0015) for _ in range(500)]
    fat = [rng.gauss(0, 0.0015) if rng.random() > 0.05 else rng.gauss(0, 0.01)
           for _ in range(500)]
    d_normal = cvar_drawdown_stop(normal, q=0.01)
    d_fat = cvar_drawdown_stop(fat, q=0.01)
    assert d_fat > d_normal
    # Both should be positive
    assert d_normal > 0
