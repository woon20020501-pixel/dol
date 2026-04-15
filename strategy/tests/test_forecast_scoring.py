"""Tests for strategy.forecast_scoring — α-cascade scoring rule."""
import math

import pytest

from strategy.forecast_scoring import (
    CascadeConfig,
    cascade_score,
    cascade_score_components,
    BaselineRing,
    tail_deterioration_flag,
    TailFlag,
)


def test_cascade_config_defaults_uniform_weights():
    c = CascadeConfig()
    assert c.alpha_grid() == (1.0, 1.5, 2.0, 2.5, 3.0)
    assert len(c.weights) == 5
    assert sum(c.weights) == pytest.approx(1.0)
    assert all(w == pytest.approx(0.2) for w in c.weights)


def test_cascade_config_rejects_subunit_alpha():
    with pytest.raises(ValueError):
        CascadeConfig(alpha_0=0.5)


def test_cascade_config_rejects_nonpositive_eta():
    with pytest.raises(ValueError):
        CascadeConfig(eta=0.0)


def test_cascade_config_rejects_weights_not_summing_to_one():
    with pytest.raises(ValueError):
        CascadeConfig(L_max=2, weights=(0.1, 0.1, 0.1))


def test_cascade_config_rejects_negative_weight():
    with pytest.raises(ValueError):
        CascadeConfig(L_max=2, weights=(0.5, -0.2, 0.7))


def test_cascade_score_empty_residuals_is_zero():
    c = CascadeConfig()
    assert cascade_score([], c) == 0.0


def test_cascade_score_negatively_oriented():
    c = CascadeConfig()
    s = cascade_score([0.3, -0.2, 0.1], c)
    assert s <= 0.0


def test_cascade_score_zero_residuals_is_zero():
    c = CascadeConfig()
    assert cascade_score([0.0, 0.0, 0.0], c) == 0.0


def test_cascade_score_monotone_in_residual_magnitude():
    c = CascadeConfig()
    small = cascade_score([0.1, 0.1], c)
    big = cascade_score([1.0, 1.0], c)
    # Larger errors → more negative score
    assert big < small


def test_cascade_score_sign_symmetry():
    c = CascadeConfig()
    pos = cascade_score([0.3, 0.5, 0.2], c)
    neg = cascade_score([-0.3, -0.5, -0.2], c)
    assert pos == pytest.approx(neg)


def test_cascade_components_sum_to_total():
    c = CascadeConfig()
    total, parts = cascade_score_components([0.4, -0.3, 0.1], c)
    assert sum(parts) == pytest.approx(total)


def test_cascade_components_empty_residuals():
    c = CascadeConfig()
    total, parts = cascade_score_components([], c)
    assert total == 0.0
    assert parts == (0.0, 0.0, 0.0, 0.0, 0.0)


# ---------------------------------------------------------------------------
# BaselineRing + tail flag
# ---------------------------------------------------------------------------

def test_baseline_ring_push_bounded_by_window():
    r = BaselineRing(window=5)
    for i in range(10):
        r.push(float(i))
    assert len(r.scores) == 5
    assert list(r.scores) == [5.0, 6.0, 7.0, 8.0, 9.0]


def test_baseline_ring_mean_std_empty_returns_zeros():
    r = BaselineRing(window=10)
    assert r.mean_std() == (0.0, 0.0)


def test_baseline_ring_mean_std_single_sample_std_zero():
    r = BaselineRing(window=10)
    r.push(3.0)
    m, s = r.mean_std()
    assert m == 3.0 and s == 0.0


def test_baseline_ring_is_ready_threshold():
    r = BaselineRing(window=100)
    assert not r.is_ready(min_samples=10)
    for i in range(10):
        r.push(float(i))
    assert r.is_ready(min_samples=10)


def test_tail_deterioration_flag_cold_start_does_not_fire():
    r = BaselineRing(window=60)
    r.push(-1.0)
    flag = tail_deterioration_flag(-10.0, r, theta_sigma=2.0, min_samples=10)
    assert flag.fired is False
    assert math.isnan(flag.z)


def test_tail_deterioration_flag_fires_on_sharp_drop():
    r = BaselineRing(window=60)
    for _ in range(30):
        r.push(-1.0)
    # Add tiny jitter so std > 0
    for i in range(30):
        r.push(-1.0 + (0.01 if i % 2 == 0 else -0.01))
    flag = tail_deterioration_flag(-10.0, r, theta_sigma=2.0, min_samples=10)
    assert flag.fired is True
    assert flag.z < -2.0


def test_tail_deterioration_flag_does_not_fire_on_improvement():
    r = BaselineRing(window=60)
    for i in range(30):
        r.push(-5.0 + (0.1 if i % 2 == 0 else -0.1))
    flag = tail_deterioration_flag(-1.0, r, theta_sigma=2.0, min_samples=10)
    # Current BETTER than baseline → not fired
    assert flag.fired is False
    assert flag.delta > 0
