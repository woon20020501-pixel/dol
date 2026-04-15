"""Tests for strategy.frontier — conformal, Bernstein, DRO, Hurst, Hawkes, OU tail."""
import math
import random
import statistics

import pytest

from strategy.frontier import (
    ConformalInterval,
    conformal_interval,
    conformal_lower_var,
    empirical_bernstein_radius,
    empirical_bernstein_credibility,
    dro_epsilon_from_sample,
    dro_tangency_weights,
    dro_objective_value,
    hurst_dfa,
    fit_hawkes,
    hawkes_log_likelihood,
    expected_cluster_size,
    ou_subgaussian_tail_bound,
    ou_tail_quantile,
)


# ---------------------------------------------------------------------------
# §1 conformal
# ---------------------------------------------------------------------------

def test_conformal_interval_empty_calibration_returns_point():
    ci = conformal_interval(0.42, [], alpha=0.05)
    assert ci.lower == 0.42 and ci.upper == 0.42
    assert ci.n_calibration == 0


def test_conformal_interval_is_symmetric_around_point():
    residuals = [-0.5, -0.2, 0.1, 0.4, 0.9, -0.3, 0.2, 0.6, -0.1, 0.5]
    ci = conformal_interval(1.0, residuals, alpha=0.2)
    assert ci.upper - 1.0 == pytest.approx(1.0 - ci.lower)


def test_conformal_interval_width_grows_with_smaller_alpha():
    residuals = [abs(random.Random(i).gauss(0, 1)) for i in range(100)]
    wide = conformal_interval(0.0, residuals, alpha=0.01)
    narrow = conformal_interval(0.0, residuals, alpha=0.50)
    assert (wide.upper - wide.lower) > (narrow.upper - narrow.lower)


def test_conformal_lower_var_matches_interval_lower():
    residuals = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]
    lb = conformal_lower_var(2.0, residuals, alpha=0.1)
    ci = conformal_interval(2.0, residuals, alpha=0.1)
    assert lb == pytest.approx(ci.lower)


def test_conformal_coverage_on_gaussian_holdout_near_target():
    rng = random.Random(123)
    cal = [rng.gauss(0, 1) for _ in range(500)]
    covered = 0
    for _ in range(500):
        true_y = rng.gauss(0, 1)
        ci = conformal_interval(0.0, cal, alpha=0.10)
        if ci.lower <= true_y <= ci.upper:
            covered += 1
    assert 0.82 <= covered / 500 <= 0.97


# ---------------------------------------------------------------------------
# §2 Maurer-Pontil Bernstein
# ---------------------------------------------------------------------------

def test_bernstein_radius_infinite_for_n_lt_2():
    assert empirical_bernstein_radius([0.5], delta=1e-6) == float("inf")
    assert empirical_bernstein_radius([], delta=1e-6) == float("inf")


def test_bernstein_radius_shrinks_as_n_grows():
    rng = random.Random(5)
    small = [rng.gauss(0, 0.1) for _ in range(50)]
    large = [rng.gauss(0, 0.1) for _ in range(5000)]
    r_small = empirical_bernstein_radius(small, delta=1e-3, value_range=1.0)
    r_large = empirical_bernstein_radius(large, delta=1e-3, value_range=1.0)
    assert r_large < r_small


def test_bernstein_credibility_interval_contains_sample_mean():
    rng = random.Random(7)
    sample = [rng.gauss(0.05, 0.2) for _ in range(400)]
    mean = statistics.mean(sample)
    lo, hi, rad = empirical_bernstein_credibility(sample, delta=1e-6)
    assert lo <= mean <= hi
    assert rad > 0


def test_bernstein_radius_scales_with_log_delta():
    rng = random.Random(9)
    sample = [rng.gauss(0, 0.1) for _ in range(200)]
    r_tight = empirical_bernstein_radius(sample, delta=1e-9, value_range=1.0)
    r_loose = empirical_bernstein_radius(sample, delta=1e-3, value_range=1.0)
    assert r_tight > r_loose  # tighter delta → larger radius


# ---------------------------------------------------------------------------
# §3 DRO
# ---------------------------------------------------------------------------

def test_dro_epsilon_decays_like_one_over_sqrt_n():
    eps_100 = dro_epsilon_from_sample(100)
    eps_10000 = dro_epsilon_from_sample(10000)
    # sqrt(100) shrinkage
    assert eps_100 / eps_10000 == pytest.approx(math.sqrt(100), rel=0.05)


def test_dro_tangency_empty_returns_empty():
    assert dro_tangency_weights([], [], 0.04) == []


def test_dro_tangency_weights_are_positive_for_positive_excess():
    mu = [0.10, 0.12, 0.08]
    # Diagonal covariance (identity-scaled)
    cov = [[0.01 if i == j else 0.0 for j in range(3)] for i in range(3)]
    w = dro_tangency_weights(mu, cov, r_idle=0.04, risk_aversion=2.0, dro_epsilon=0.04)
    assert len(w) == 3
    assert all(wi > 0 for wi in w)


def test_dro_objective_value_penalises_concentration():
    mu = [0.10, 0.10]
    cov = [[0.01, 0.0], [0.0, 0.01]]
    diversified = [0.5, 0.5]
    concentrated = [1.0, 0.0]
    # With a positive DRO ε, concentrated solution has larger L2 norm,
    # so its penalty is larger. Means are equal, so diversified should win.
    v_div = dro_objective_value(diversified, mu, cov, r_idle=0.04,
                                risk_aversion=2.0, dro_epsilon=0.05)
    v_con = dro_objective_value(concentrated, mu, cov, r_idle=0.04,
                                risk_aversion=2.0, dro_epsilon=0.05)
    assert v_div > v_con


# ---------------------------------------------------------------------------
# §4 Hurst DFA
# ---------------------------------------------------------------------------

def test_hurst_returns_none_on_short_series():
    assert hurst_dfa([0.1] * 20) is None


def test_hurst_random_walk_near_half():
    rng = random.Random(0)
    rw_increments = [rng.gauss(0, 1) for _ in range(1200)]
    h = hurst_dfa(rw_increments)
    assert h is not None
    # Increments of a random walk are iid → H ≈ 0.5
    assert 0.30 <= h <= 0.70


# ---------------------------------------------------------------------------
# §5 Hawkes
# ---------------------------------------------------------------------------

def test_hawkes_returns_none_on_sparse_events():
    assert fit_hawkes([1.0, 2.0], T=10.0) is None


def test_hawkes_log_likelihood_invalid_params_returns_neg_inf():
    assert hawkes_log_likelihood([1.0, 2.0], 10.0, mu_0=0.0, alpha=0.5, beta=1.0) == -float("inf")
    assert hawkes_log_likelihood([1.0, 2.0], 10.0, mu_0=0.5, alpha=-0.1, beta=1.0) == -float("inf")


def test_hawkes_log_likelihood_zero_events_is_baseline_compensator():
    ll = hawkes_log_likelihood([], T=10.0, mu_0=0.5, alpha=0.3, beta=1.0)
    assert ll == pytest.approx(-0.5 * 10.0)


def test_expected_cluster_size_stationary_and_critical():
    assert expected_cluster_size(0.0) == 1.0
    assert expected_cluster_size(0.5) == 2.0
    assert expected_cluster_size(0.8) == pytest.approx(5.0)
    assert expected_cluster_size(1.0) == float("inf")


# ---------------------------------------------------------------------------
# §6 OU sub-Gaussian tail
# ---------------------------------------------------------------------------

def test_ou_subgaussian_tail_bound_values():
    assert ou_subgaussian_tail_bound(1.0, 0) == pytest.approx(2.0)
    # 1-sigma bound = 2 exp(-1/2) ≈ 1.213
    assert ou_subgaussian_tail_bound(1.0, 1.0) == pytest.approx(2 * math.exp(-0.5))
    # Monotone decreasing in x
    assert (ou_subgaussian_tail_bound(1.0, 3) <
            ou_subgaussian_tail_bound(1.0, 2) <
            ou_subgaussian_tail_bound(1.0, 1))


def test_ou_tail_quantile_is_inverse_of_bound():
    for alpha in [0.05, 0.01, 0.001]:
        x = ou_tail_quantile(1.0, alpha)
        # Plugging back in should recover alpha
        assert ou_subgaussian_tail_bound(1.0, x) == pytest.approx(alpha)


def test_ou_tail_quantile_scales_with_stationary_std():
    q1 = ou_tail_quantile(1.0, 0.05)
    q2 = ou_tail_quantile(2.0, 0.05)
    assert q2 == pytest.approx(2.0 * q1)
