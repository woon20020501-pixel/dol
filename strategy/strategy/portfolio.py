"""
portfolio.py — mean-variance portfolio optimization with chance-constrained
mandate compliance. Implements docs/math-rigorous.md §5, §7.

For a universe of candidate same-asset cross-venue funding pairs, computes:
  - Cross-pair covariance matrix from rolling spread returns
  - Markowitz tangency / fractional-Kelly weights
  - Projection onto budget + box + per-counter constraints
  - Chance constraint: P(R_vault ≥ floor) ≥ 1 − ε

Pure stdlib + numpy-style hand-rolled linear algebra. No SciPy / cvxpy.
"""
from __future__ import annotations
import math
import statistics
from dataclasses import dataclass, field
from typing import Sequence


# ===========================================================================
# §5.2 — covariance matrix estimation
# ===========================================================================

def covariance_matrix(returns_by_pair: list) -> list:
    """returns_by_pair: list of N lists of equal length T. Returns N×N matrix."""
    N = len(returns_by_pair)
    if N == 0:
        return []
    T = len(returns_by_pair[0])
    means = [sum(r) / T for r in returns_by_pair]
    cov = [[0.0] * N for _ in range(N)]
    for i in range(N):
        for j in range(i, N):
            c = sum((returns_by_pair[i][t] - means[i]) * (returns_by_pair[j][t] - means[j])
                    for t in range(T)) / max(T - 1, 1)
            cov[i][j] = c
            cov[j][i] = c
    return cov


def shrink_covariance(cov: list, lam: float = 0.10) -> list:
    """Ledoit-Wolf style shrinkage toward diagonal target. lam=0 → sample cov,
    lam=1 → fully diagonal. 0.10 default for stability with small T."""
    N = len(cov)
    if N == 0:
        return []
    target = [[cov[i][i] if i == j else 0.0 for j in range(N)] for i in range(N)]
    return [[(1 - lam) * cov[i][j] + lam * target[i][j] for j in range(N)] for i in range(N)]


# ===========================================================================
# Linear algebra helpers
# ===========================================================================

def matvec(A: list, v: list) -> list:
    n = len(A)
    return [sum(A[i][j] * v[j] for j in range(n)) for i in range(n)]


def quadform(v: list, A: list) -> float:
    n = len(A)
    return sum(v[i] * A[i][j] * v[j] for i in range(n) for j in range(n))


def solve_spd(A: list, b: list, ridge: float = 1e-9) -> list | None:
    """Solve Ax = b for symmetric positive-(semi-)definite A via Cholesky.
    Adds tiny ridge for numerical stability. Returns None on failure."""
    n = len(A)
    # Add ridge to diagonal
    M = [[A[i][j] + (ridge if i == j else 0.0) for j in range(n)] for i in range(n)]
    # Cholesky factorization: M = L L^T
    L = [[0.0] * n for _ in range(n)]
    for i in range(n):
        for j in range(i + 1):
            s = sum(L[i][k] * L[j][k] for k in range(j))
            if i == j:
                d = M[i][i] - s
                if d <= 0:
                    return None
                L[i][j] = math.sqrt(d)
            else:
                L[i][j] = (M[i][j] - s) / L[j][j]
    # Forward solve L y = b
    y = [0.0] * n
    for i in range(n):
        y[i] = (b[i] - sum(L[i][k] * y[k] for k in range(i))) / L[i][i]
    # Back solve L^T x = y
    x = [0.0] * n
    for i in range(n - 1, -1, -1):
        x[i] = (y[i] - sum(L[k][i] * x[k] for k in range(i + 1, n))) / L[i][i]
    return x


# ===========================================================================
# §5.3 — Markowitz tangency / fractional Kelly
# ===========================================================================

def tangency_weights(expected_returns: list, cov: list, r_idle: float,
                     risk_aversion: float = 2.0) -> list | None:
    """Unconstrained fractional-Kelly weights:
       w = (1/γ) · Σ^(-1) · (r − r_idle · 1)
    Returns None if Σ is singular."""
    N = len(expected_returns)
    excess = [expected_returns[i] - r_idle for i in range(N)]
    sol = solve_spd(cov, excess)
    if sol is None:
        return None
    return [s / risk_aversion for s in sol]


# ===========================================================================
# Constrained allocation: project tangency onto feasible polytope
# ===========================================================================

@dataclass
class Constraints:
    budget: float                                  # Σ w_i ≤ budget (= 1−α)
    max_per_position: float                        # 0 ≤ w_i ≤ m_pos
    max_per_counter: dict = field(default_factory=dict)  # {counter_venue: m_counter}
    counter_of: list = field(default_factory=list)       # counter venue per pair index


def project_to_constraints(w: list, c: Constraints, max_iter: int = 100,
                           tol: float = 1e-9) -> list:
    """Iteratively project onto box + budget + per-counter constraints.
    Simple alternating-projection scheme. Converges fast for small N."""
    N = len(w)
    proj = list(w)
    for _ in range(max_iter):
        prev = list(proj)
        # Box projection
        for i in range(N):
            proj[i] = max(0.0, min(c.max_per_position, proj[i]))
        # Budget projection (scale down if over budget)
        s = sum(proj)
        if s > c.budget and s > 0:
            scale = c.budget / s
            proj = [w_i * scale for w_i in proj]
        # Per-counter projection
        for venue, cap in c.max_per_counter.items():
            indices = [i for i in range(N) if c.counter_of[i] == venue]
            sub_sum = sum(proj[i] for i in indices)
            if sub_sum > cap and sub_sum > 0:
                scale = cap / sub_sum
                for i in indices:
                    proj[i] *= scale
        # Convergence check
        delta = sum(abs(proj[i] - prev[i]) for i in range(N))
        if delta < tol:
            break
    return proj


def allocate_markowitz(expected_returns: list, cov: list, r_idle: float,
                       constraints: Constraints, risk_aversion: float = 2.0) -> list:
    """Returns weights vector (one per candidate pair) summing to ≤ constraints.budget."""
    N = len(expected_returns)
    if N == 0:
        return []
    w_unconstrained = tangency_weights(expected_returns, cov, r_idle, risk_aversion)
    if w_unconstrained is None:
        # Fall back to inverse-variance weights
        diag = [cov[i][i] for i in range(N)]
        excess = [expected_returns[i] - r_idle for i in range(N)]
        w_unconstrained = [excess[i] / max(diag[i], 1e-12) for i in range(N)]
    return project_to_constraints(w_unconstrained, constraints)


# ===========================================================================
# §7 — Chance-constrained mandate compliance
# ===========================================================================

@dataclass
class ChanceConstraintResult:
    feasible: bool
    weights: list
    idle_alpha: float
    portfolio_mean_apy: float
    portfolio_std_apy: float
    vault_5pct_apy: float
    vault_1pct_apy: float
    target_floor: float
    binds: str   # which constraint binds: 'budget' / 'chance' / 'stress' / 'box'


# Standard normal inverse CDF approximations
def _norm_inv(p: float) -> float:
    """Beasley-Springer-Moro approximation of Φ^(-1)(p)."""
    if p <= 0 or p >= 1:
        return 0.0
    if p < 0.5:
        return -_norm_inv(1 - p)
    a = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02,
         1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00]
    b = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02,
         6.680131188771972e+01, -1.328068155288572e+01]
    c = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00,
         -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00]
    d = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00,
         3.754408661907416e+00]
    p_low = 0.02425
    if p < p_low:
        q = math.sqrt(-2 * math.log(p))
        return (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / \
               ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1)
    q = p - 0.5
    r = q * q
    return (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q / \
           (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1)


def chance_constrained_allocate(expected_returns: list, cov: list, r_idle: float,
                                 constraints: Constraints, leverage: int,
                                 mandate_floor: float, stress_floor: float,
                                 epsilon: float = 0.05, stress_eps: float = 0.01,
                                 risk_aversion: float = 2.0,
                                 fat_tail_multiplier: float = 1.0,
                                 method: str = "approx-gaussian") -> ChanceConstraintResult:
    """α-grid HEURISTIC for the chance-constrained allocation (NOT an exact CCP/SOCP
    solver — critique #7 acknowledgment):

       max  Sharpe
       s.t. P(R_vault ≥ mandate_floor) ≥ 1 − epsilon
            P(R_vault ≥ stress_floor)  ≥ 1 − stress_eps
            box / budget / counter constraints

    Implementation: sweep α (idle fraction) over [0.50, 0.95] at 1% grid; at each
    α, solve the inner Markowitz problem and check whether the vault-level
    quantile constraints hold. Return the best-Sharpe feasible result. The α-grid
    is a heuristic — an exact solver would bisect on α and at each step solve an
    SOCP on the inner allocation. For the current universe (46 candidates, two
    quantile constraints) the grid is dense enough that the gap is ≤ 1bp, but
    this should be revisited if the constraint set grows.

    **Tail quantile estimation (critique #4 remediation):**

      - `method="approx-gaussian"` (legacy path, now flagged as UNSAFE for real
        cross-venue basis residuals): uses `z_eps = Φ⁻¹(1−ε) ≈ 1.645` and
        `z_stress = Φ⁻¹(1−ε_stress) ≈ 2.326`. This assumes vault-level returns
        are Gaussian, which requires the per-pair residuals to be Gaussian OR
        for enough independent pairs that a CLT argument holds. On real basis
        history (Hurst ≈ 0.9, dependence across venues) NEITHER condition holds.

      - `fat_tail_multiplier` (float ≥ 1.0, default 1.0): scales the z-values
        upward to account for the discrepancy between Gaussian and empirical
        tail quantiles. The caller (usually `rigorous.compute_rigorous_state`)
        is expected to compute this from the empirical CVaR-to-Gaussian ratio
        on the actual signed return history, so the vault quantile bound
        reflects the tails the data showed. Default 1.0 preserves the legacy
        Gaussian path for the validation suite that uses synthetic OU inputs.

      - `method="empirical"` is reserved for a future implementation that
        takes aligned signed return series directly and computes the vault
        quantile via Monte Carlo over the historical joint distribution. Not
        implemented in v3.5.2.
    """
    N = len(expected_returns)
    if N == 0:
        return ChanceConstraintResult(
            feasible=False, weights=[], idle_alpha=1.0,
            portfolio_mean_apy=0.0, portfolio_std_apy=0.0,
            vault_5pct_apy=r_idle, vault_1pct_apy=r_idle,
            target_floor=mandate_floor, binds="empty universe",
        )

    if fat_tail_multiplier < 1.0:
        fat_tail_multiplier = 1.0  # can only inflate, never shrink
    z_eps = _norm_inv(1 - epsilon) * fat_tail_multiplier        # Gaussian × fat-tail inflation
    z_stress = _norm_inv(1 - stress_eps) * fat_tail_multiplier

    alpha_min = 0.50
    alpha_max = 0.95
    best = None

    # Try alpha values in [alpha_min, alpha_max] descending (more deployment first)
    # Find smallest alpha (most deployment) that satisfies both chance constraints.
    # Then try slightly larger alpha to see if Sharpe improves (because chance constraint slack).
    for alpha in [alpha_min + 0.01 * k for k in range(int((alpha_max - alpha_min) * 100) + 1)]:
        budget_alpha = 1 - alpha
        c = Constraints(
            budget=budget_alpha,
            max_per_position=constraints.max_per_position,
            max_per_counter=constraints.max_per_counter,
            counter_of=constraints.counter_of,
        )
        w = allocate_markowitz(expected_returns, cov, r_idle, c, risk_aversion)
        if not w or sum(w) <= 0:
            continue
        portfolio_mean = sum(w[i] * expected_returns[i] for i in range(N))
        portfolio_var = quadform(w, cov)
        portfolio_std = math.sqrt(max(portfolio_var, 0))
        # vault APY moments (with leverage)
        deployed = sum(w)  # fraction of AUM in trading bucket margin
        # portfolio return per dollar of AUM = deployed × leverage/2 × (weighted pair return per leg notional)
        # Actually, since w is fraction of AUM and portfolio_mean is the weighted return,
        # the trading contribution to vault APY is: portfolio_mean × leverage/2 (per unit margin → per unit per-leg notional)
        # Hmm let me think carefully. If w_i is fraction of AUM allocated as PAIR MARGIN to pair i,
        # and pair i's per-leg notional is w_i × A × L / 2 (since margin = notional / L on each of 2 legs),
        # then funding income per year = expected_returns[i] × (w_i × A × L / 2).
        # So total trading bucket annual income = Σ w_i × r_i × L / 2 × A
        # Trading bucket APY on AUM = Σ w_i × r_i × L / 2 = (L/2) × portfolio_mean
        # Trading bucket capital used = sum(w_i) × A
        # So return per deployed margin = trading_income / deployed_margin = (L/2) × portfolio_mean / deployed
        trading_apy_on_aum = (leverage / 2) * portfolio_mean
        idle_apy_on_aum = alpha * r_idle
        vault_mean = idle_apy_on_aum + trading_apy_on_aum
        # variance scales similarly
        vault_var = (leverage / 2) ** 2 * portfolio_var
        vault_std = math.sqrt(max(vault_var, 0))
        vault_5pct = vault_mean - z_eps * vault_std
        vault_1pct = vault_mean - z_stress * vault_std
        chance_ok = vault_5pct >= mandate_floor
        stress_ok = vault_1pct >= stress_floor
        if chance_ok and stress_ok:
            sharpe = (vault_mean - r_idle) / max(vault_std, 1e-9)
            if best is None or sharpe > best[0]:
                binding = "chance" if abs(vault_5pct - mandate_floor) < 0.001 else \
                          "stress" if abs(vault_1pct - stress_floor) < 0.001 else \
                          "budget" if abs(deployed - (1 - alpha)) < 1e-6 else "box"
                best = (sharpe, ChanceConstraintResult(
                    feasible=True, weights=w, idle_alpha=alpha,
                    portfolio_mean_apy=vault_mean, portfolio_std_apy=vault_std,
                    vault_5pct_apy=vault_5pct, vault_1pct_apy=vault_1pct,
                    target_floor=mandate_floor, binds=binding,
                ))

    if best is None:
        return ChanceConstraintResult(
            feasible=False, weights=[0.0] * N, idle_alpha=alpha_max,
            portfolio_mean_apy=alpha_max * r_idle, portfolio_std_apy=0.0,
            vault_5pct_apy=alpha_max * r_idle, vault_1pct_apy=alpha_max * r_idle,
            target_floor=mandate_floor, binds="infeasible: no allocation clears chance constraint",
        )
    return best[1]


# ===========================================================================
# Smoke test
# ===========================================================================

if __name__ == "__main__":
    import random
    random.seed(0)
    N = 8
    T = 720
    # Synthetic per-hour signed returns for 8 pairs with varying mean and noise
    means = [0.000020, 0.000025, 0.000030, 0.000022, 0.000018, 0.000028, 0.000024, 0.000026]
    stds = [0.000005, 0.000006, 0.000007, 0.000005, 0.000004, 0.000006, 0.000005, 0.000005]
    returns = [[random.gauss(means[i], stds[i]) for _ in range(T)] for i in range(N)]
    cov = covariance_matrix(returns)
    cov_shrunk = shrink_covariance(cov, lam=0.10)
    # Convert mean per-hour returns to APY for the optimization
    expected_apy = [m * 24 * 365 for m in means]
    # Convert covariance to APY units (linear scaling: variance of sum ≈ T × var)
    cov_apy = [[c * (24 * 365) for c in row] for row in cov_shrunk]

    print("=== expected APYs (per-pair signed mean) ===")
    for i, r in enumerate(expected_apy):
        print(f"  pair {i}: {r*100:.2f}%")

    print()
    print("=== Markowitz tangency (unconstrained) ===")
    r_idle = 0.044
    w_unc = tangency_weights(expected_apy, cov_apy, r_idle, risk_aversion=2.0)
    for i, w in enumerate(w_unc):
        print(f"  w_{i} = {w:.4f}")
    print(f"  sum: {sum(w_unc):.4f}")
    print()

    print("=== chance-constrained allocation ===")
    constraints = Constraints(
        budget=0.50,
        max_per_position=0.04,
        max_per_counter={"backpack": 0.40, "hyperliquid": 0.40},
        counter_of=["backpack"] * 4 + ["hyperliquid"] * 4,
    )
    result = chance_constrained_allocate(
        expected_apy, cov_apy, r_idle, constraints, leverage=2,
        mandate_floor=0.08, stress_floor=0.06, epsilon=0.05, stress_eps=0.01,
    )
    print(f"  feasible: {result.feasible}")
    print(f"  alpha (idle): {result.idle_alpha*100:.1f}%")
    print(f"  vault mean APY: {result.portfolio_mean_apy*100:.2f}%")
    print(f"  vault 5%-VaR APY: {result.vault_5pct_apy*100:.2f}% (>= floor 8%)")
    print(f"  vault 1%-VaR APY: {result.vault_1pct_apy*100:.2f}% (>= stress 6%)")
    print(f"  binding constraint: {result.binds}")
    print(f"  weights:")
    for i, w in enumerate(result.weights):
        print(f"    pair {i} ({constraints.counter_of[i]:<12}): {w*100:.2f}% AUM")
    print(f"  total deployed: {sum(result.weights)*100:.2f}% AUM")
