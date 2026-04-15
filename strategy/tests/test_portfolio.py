"""Tests for strategy.portfolio — covariance, Markowitz, chance-constrained."""
import math
import random

import pytest

from strategy.portfolio import (
    covariance_matrix,
    shrink_covariance,
    matvec,
    quadform,
    solve_spd,
    tangency_weights,
    Constraints,
    project_to_constraints,
    allocate_markowitz,
    chance_constrained_allocate,
    _norm_inv,
)


def test_covariance_matrix_dimensions_and_symmetry():
    rng = random.Random(0)
    returns = [[rng.gauss(0, 1) for _ in range(100)] for _ in range(4)]
    cov = covariance_matrix(returns)
    assert len(cov) == 4
    assert len(cov[0]) == 4
    for i in range(4):
        for j in range(4):
            assert cov[i][j] == pytest.approx(cov[j][i])


def test_covariance_diagonal_is_variance():
    rng = random.Random(1)
    x = [rng.gauss(0, 2.0) for _ in range(500)]
    cov = covariance_matrix([x])
    import statistics
    assert cov[0][0] == pytest.approx(statistics.variance(x), rel=1e-9)


def test_shrink_covariance_identity_when_lam_zero():
    cov = [[1.0, 0.5], [0.5, 2.0]]
    assert shrink_covariance(cov, lam=0.0) == cov


def test_shrink_covariance_diagonal_when_lam_one():
    cov = [[1.0, 0.5], [0.5, 2.0]]
    out = shrink_covariance(cov, lam=1.0)
    assert out == [[1.0, 0.0], [0.0, 2.0]]


def test_shrink_covariance_empty():
    assert shrink_covariance([], lam=0.1) == []


def test_matvec_and_quadform():
    A = [[2.0, 0.0], [0.0, 3.0]]
    v = [4.0, 5.0]
    assert matvec(A, v) == [8.0, 15.0]
    assert quadform(v, A) == pytest.approx(2 * 16 + 3 * 25)


def test_solve_spd_identity_returns_b():
    I = [[1.0, 0.0], [0.0, 1.0]]
    assert solve_spd(I, [3.0, 4.0]) == pytest.approx([3.0, 4.0])


def test_solve_spd_2x2():
    A = [[4.0, 1.0], [1.0, 3.0]]
    b = [1.0, 2.0]
    x = solve_spd(A, b)
    # Ax should recover b
    recon = matvec(A, x)
    assert recon[0] == pytest.approx(1.0, abs=1e-6)
    assert recon[1] == pytest.approx(2.0, abs=1e-6)


def test_tangency_weights_diagonal_inverse_variance():
    mu = [0.10, 0.12]
    cov = [[0.01, 0.0], [0.0, 0.04]]
    w = tangency_weights(mu, cov, r_idle=0.04, risk_aversion=2.0)
    # w_i = (mu_i - r_idle) / (γ * σ²)
    assert w[0] == pytest.approx((0.10 - 0.04) / (2.0 * 0.01))
    assert w[1] == pytest.approx((0.12 - 0.04) / (2.0 * 0.04))


def test_project_to_constraints_respects_box_and_budget():
    w = [0.5, 0.6, 0.7]
    c = Constraints(budget=0.5, max_per_position=0.25,
                    max_per_counter={}, counter_of=["a", "a", "a"])
    proj = project_to_constraints(w, c)
    assert all(0.0 <= p <= 0.25 + 1e-9 for p in proj)
    assert sum(proj) <= 0.5 + 1e-9


def test_project_to_constraints_per_counter_cap():
    w = [0.3, 0.3, 0.3]
    c = Constraints(budget=1.0, max_per_position=1.0,
                    max_per_counter={"bp": 0.4}, counter_of=["bp", "bp", "hl"])
    proj = project_to_constraints(w, c)
    bp_sum = proj[0] + proj[1]
    assert bp_sum <= 0.4 + 1e-9


def test_allocate_markowitz_empty_returns_empty():
    c = Constraints(budget=0.5, max_per_position=0.1,
                    max_per_counter={}, counter_of=[])
    assert allocate_markowitz([], [], 0.04, c) == []


def test_norm_inv_symmetry_and_monotonicity():
    # _norm_inv(0.5) should be 0
    assert _norm_inv(0.5) == pytest.approx(0.0, abs=1e-9)
    # Classic critical values
    assert _norm_inv(0.95) == pytest.approx(1.6449, abs=1e-3)
    assert _norm_inv(0.99) == pytest.approx(2.3263, abs=1e-3)
    # Monotonicity
    assert _norm_inv(0.10) < _norm_inv(0.25) < _norm_inv(0.75) < _norm_inv(0.90)


def test_chance_constrained_empty_universe():
    c = Constraints(budget=0.5, max_per_position=0.04,
                    max_per_counter={}, counter_of=[])
    res = chance_constrained_allocate(
        expected_returns=[], cov=[], r_idle=0.04,
        constraints=c, leverage=2, mandate_floor=0.08, stress_floor=0.06,
    )
    assert res.feasible is False
    assert res.weights == []
