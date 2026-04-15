"""
frontier.py — modern (2005-2024) statistical machinery for the Dol strategy.

Implements docs/math-frontier.md §1-§6 as a single self-contained module:
  - Inductive conformal prediction for distribution-free VaR
  - Maurer-Pontil empirical Bernstein concentration
  - Esfahani-Kuhn Wasserstein distributionally robust optimization
  - DFA-based Hurst exponent estimation
  - Exponential-kernel Hawkes self-exciting process MLE
  - Sub-Gaussian OU tail bound (the "Dol Theorem")

All in pure stdlib + numpy-style hand-rolled linear algebra. No scipy / cvxpy.
"""
from __future__ import annotations
import math
import statistics
from dataclasses import dataclass
from typing import Sequence


# ===========================================================================
# §1 — Inductive conformal prediction
# ===========================================================================

@dataclass
class ConformalInterval:
    point: float
    lower: float
    upper: float
    coverage_target: float
    n_calibration: int


def conformal_interval(point_prediction: float,
                       calibration_residuals: Sequence[float],
                       alpha: float = 0.05) -> ConformalInterval:
    """Inductive conformal prediction interval.

    Given a point prediction and a calibration set of nonconformity scores
    (e.g., absolute residuals from holdout), returns a prediction interval
    with finite-sample coverage 1-α under exchangeability.

    Theorem (Vovk 2005): the interval contains the true value with probability
    exactly ⌈(n+1)(1-α)⌉/(n+1) ≥ 1-α for any data distribution.
    """
    scores = sorted(abs(r) for r in calibration_residuals)
    n = len(scores)
    if n == 0:
        return ConformalInterval(point=point_prediction, lower=point_prediction,
                                 upper=point_prediction, coverage_target=1 - alpha,
                                 n_calibration=0)
    # The (n+1)(1-α) order statistic. Use ceiling, clamp to n.
    k = min(n, max(1, math.ceil((n + 1) * (1 - alpha))))
    q = scores[k - 1]
    return ConformalInterval(
        point=point_prediction, lower=point_prediction - q,
        upper=point_prediction + q, coverage_target=1 - alpha, n_calibration=n,
    )


def conformal_lower_var(point_prediction: float,
                        calibration_residuals: Sequence[float],
                        alpha: float = 0.05) -> float:
    """One-sided conformal lower bound (VaR) for the prediction."""
    return conformal_interval(point_prediction, calibration_residuals, alpha).lower


# ===========================================================================
# §2 — Maurer-Pontil empirical Bernstein bound
# ===========================================================================

def empirical_bernstein_radius(values: Sequence[float], delta: float = 1e-6,
                               value_range: float = 1.0) -> float:
    """Half-width of the empirical Bernstein confidence interval for the mean.

    For independent X_i ∈ [0, value_range] (after rescaling), with empirical
    variance V̂_n, Maurer-Pontil 2009 gives:

        |X̄_n - E[X]| ≤ √(2 V̂_n ln(2/δ) / n) + 7 · value_range · ln(2/δ) / (3(n-1))

    with probability ≥ 1-δ. Returns the half-width.

    For δ = 1e-6 this is the 5σ-equivalent test (one-sided ~6e-7).
    """
    n = len(values)
    if n < 2:
        return float("inf")
    var = statistics.variance(values)
    log_term = math.log(2.0 / delta)
    term1 = math.sqrt(2.0 * var * log_term / n)
    term2 = 7.0 * value_range * log_term / (3.0 * (n - 1))
    return term1 + term2


def empirical_bernstein_credibility(values: Sequence[float],
                                    delta: float = 1e-6) -> tuple:
    """Returns (lower_bound_on_mean, upper_bound_on_mean, half_width).
    Auto-rescales to [0, max(|values|)] for a safe value_range."""
    if len(values) < 2:
        return (0.0, 0.0, float("inf"))
    mean = statistics.mean(values)
    max_abs = max(abs(v) for v in values)
    if max_abs == 0:
        return (mean, mean, 0.0)
    radius = empirical_bernstein_radius(values, delta=delta, value_range=2 * max_abs)
    return (mean - radius, mean + radius, radius)


# ===========================================================================
# §3 — Wasserstein distributionally robust portfolio
# ===========================================================================

@dataclass
class DRORegularization:
    epsilon: float                 # Wasserstein ball radius
    n_calibration: int             # sample size used to derive ε


def dro_epsilon_from_sample(n: int, confidence: float = 0.95,
                            diameter_estimate: float = 0.10) -> float:
    """Esfahani-Kuhn 2018 Theorem 3.4: optimal Wasserstein radius for finite-sample
    out-of-sample guarantee at confidence 1-η is

        ε_n = D · √(log(C / η) / n)

    where D is the diameter of the support and C is a constant (we use 2).
    Decays as O(1/√n)."""
    eta = 1 - confidence
    return diameter_estimate * math.sqrt(math.log(2.0 / eta) / max(n, 1))


def dro_objective_value(weights: Sequence[float], expected_returns: Sequence[float],
                        covariance: Sequence[Sequence[float]], r_idle: float,
                        risk_aversion: float, dro_epsilon: float) -> float:
    """The Wasserstein-DRO mean-variance objective:

        max_w  w'r̂ - r_idle - λ √(w'Σ̂w) - ε ||w||_2

    where the ε term is the DRO regularization. Returns the objective value
    for given weights."""
    n = len(weights)
    excess = sum(weights[i] * (expected_returns[i] - r_idle) for i in range(n))
    var = sum(weights[i] * covariance[i][j] * weights[j]
              for i in range(n) for j in range(n))
    std = math.sqrt(max(var, 0))
    norm = math.sqrt(sum(w * w for w in weights))
    return excess - risk_aversion * std - dro_epsilon * norm


def dro_tangency_weights(expected_returns: Sequence[float],
                         covariance: Sequence[Sequence[float]],
                         r_idle: float,
                         risk_aversion: float = 2.0,
                         dro_epsilon: float = 0.04) -> list:
    """Approximate Wasserstein-DRO tangency weights via penalized linear system.

    The DRO problem with mean-variance utility and L2 Wasserstein cost reduces to:
        (Σ + ε² I + ε/√(w'Σw) Σ) w = (1/γ)(r - r_idle 1)
    For practical use, we solve the proxy:
        (Σ + ε I) w = (1/γ)(r - r_idle 1)
    which is ridge-regularized Markowitz. ε plays the role of distributional
    uncertainty; ridge = noise penalty. The shrinkage equivalence is exact for
    spherical covariance and approximately optimal otherwise.
    """
    n = len(expected_returns)
    if n == 0:
        return []
    excess = [expected_returns[i] - r_idle for i in range(n)]
    # Σ + ε I
    M = [[covariance[i][j] + (dro_epsilon if i == j else 0.0) for j in range(n)] for i in range(n)]
    sol = _cholesky_solve(M, excess)
    if sol is None:
        # singular: fall back to scaled excess
        return [e / max(1.0, dro_epsilon) for e in excess]
    return [s / risk_aversion for s in sol]


def _cholesky_solve(A: list, b: list, ridge: float = 1e-9) -> list:
    """Cholesky solve for symmetric PD matrix. Adds tiny ridge for stability."""
    n = len(A)
    M = [[A[i][j] + (ridge if i == j else 0.0) for j in range(n)] for i in range(n)]
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
    y = [0.0] * n
    for i in range(n):
        y[i] = (b[i] - sum(L[i][k] * y[k] for k in range(i))) / L[i][i]
    x = [0.0] * n
    for i in range(n - 1, -1, -1):
        x[i] = (y[i] - sum(L[k][i] * x[k] for k in range(i + 1, n))) / L[i][i]
    return x


# ===========================================================================
# §4 — Hurst exponent via DFA
# ===========================================================================

def hurst_dfa(series: Sequence[float], min_window: int = 10,
              max_window: int | None = None) -> float | None:
    """Detrended fluctuation analysis (Peng et al. 1994). Returns Hurst exponent.

    H = 0.5 → uncorrelated random walk
    H < 0.5 → anti-persistent / mean-reverting / rough
    H > 0.5 → persistent / trending"""
    n = len(series)
    if n < 4 * min_window:
        return None
    if max_window is None:
        max_window = n // 4
    mean = statistics.mean(series)
    Y = []
    cum = 0.0
    for s in series:
        cum += s - mean
        Y.append(cum)

    # Window sizes log-spaced
    sizes = []
    w = min_window
    while w <= max_window:
        sizes.append(w)
        w = int(w * 1.3) + 1
    if len(sizes) < 4:
        return None

    log_n = []
    log_F = []
    for win in sizes:
        n_windows = n // win
        if n_windows < 2:
            continue
        F2_total = 0.0
        for k in range(n_windows):
            chunk = Y[k * win:(k + 1) * win]
            # Linear detrend
            xs = list(range(win))
            x_mean = (win - 1) / 2.0
            y_mean = sum(chunk) / win
            num = sum((xs[i] - x_mean) * (chunk[i] - y_mean) for i in range(win))
            den = sum((xs[i] - x_mean) ** 2 for i in range(win))
            slope = num / den if den != 0 else 0.0
            intercept = y_mean - slope * x_mean
            for i in range(win):
                resid = chunk[i] - (slope * xs[i] + intercept)
                F2_total += resid * resid
        F = math.sqrt(F2_total / (n_windows * win))
        if F > 0:
            log_n.append(math.log(win))
            log_F.append(math.log(F))

    if len(log_n) < 4:
        return None
    # Linear regression slope of log_F on log_n
    nn = len(log_n)
    x_bar = sum(log_n) / nn
    y_bar = sum(log_F) / nn
    num = sum((log_n[i] - x_bar) * (log_F[i] - y_bar) for i in range(nn))
    den = sum((log_n[i] - x_bar) ** 2 for i in range(nn))
    if den == 0:
        return None
    return num / den


# ===========================================================================
# §5 — Exponential-kernel Hawkes process MLE
# ===========================================================================

@dataclass
class HawkesFit:
    mu_0: float       # baseline intensity
    alpha: float      # branching ratio (avg children per event)
    beta: float       # decay rate
    log_likelihood: float
    n_events: int
    T_horizon: float
    is_stationary: bool


def hawkes_log_likelihood(events: Sequence[float], T: float,
                          mu_0: float, alpha: float, beta: float) -> float:
    """Log-likelihood for exponential-kernel Hawkes."""
    if mu_0 <= 0 or alpha < 0 or beta <= 0:
        return -float("inf")
    n = len(events)
    if n == 0:
        return -mu_0 * T
    # Σ log λ(t_i) — use recursive intensity update
    log_int_sum = 0.0
    A = 0.0  # accumulator
    last_t = 0.0
    for i, t in enumerate(events):
        if i == 0:
            lam = mu_0
        else:
            A = math.exp(-beta * (t - last_t)) * (1 + A)
            lam = mu_0 + alpha * beta * A
        if lam <= 0:
            return -float("inf")
        log_int_sum += math.log(lam)
        last_t = t
    # Compensator: ∫_0^T λ(s) ds
    compensator = mu_0 * T + alpha * sum(1 - math.exp(-beta * (T - t)) for t in events)
    return log_int_sum - compensator


def fit_hawkes(events: Sequence[float], T: float, max_iter: int = 200) -> HawkesFit | None:
    """MLE fit for exponential-kernel Hawkes. Coordinate ascent over (μ_0, α, β).

    Returns None if events are too sparse."""
    if len(events) < 5:
        return None

    # Initial guesses
    mu_0 = len(events) / T
    alpha = 0.3
    beta = 1.0 / max(statistics.median([events[i+1] - events[i] for i in range(len(events)-1)]), 1e-3)

    best_ll = hawkes_log_likelihood(events, T, mu_0, alpha, beta)
    for _ in range(max_iter):
        improved = False
        # Try perturbations
        for delta in (1.5, 0.667):
            for which in ("mu", "alpha", "beta"):
                new_mu, new_alpha, new_beta = mu_0, alpha, beta
                if which == "mu":
                    new_mu = mu_0 * delta
                elif which == "alpha":
                    new_alpha = min(0.99, alpha * delta)
                else:
                    new_beta = beta * delta
                ll = hawkes_log_likelihood(events, T, new_mu, new_alpha, new_beta)
                if ll > best_ll:
                    best_ll = ll
                    mu_0, alpha, beta = new_mu, new_alpha, new_beta
                    improved = True
        if not improved:
            break

    return HawkesFit(
        mu_0=mu_0, alpha=alpha, beta=beta,
        log_likelihood=best_ll, n_events=len(events),
        T_horizon=T, is_stationary=alpha < 0.99,
    )


def expected_cluster_size(alpha: float) -> float:
    """For exponential-kernel Hawkes, the expected cluster size from one parent
    event is 1/(1-α) when α < 1 (stationary). Used to refine drawdown stops."""
    if alpha >= 1:
        return float("inf")
    return 1.0 / (1.0 - alpha)


# ===========================================================================
# §6 — Sub-Gaussian OU tail bound (the "Dol Theorem")
# ===========================================================================

def ou_subgaussian_tail_bound(stationary_std: float, x_sigmas: float) -> float:
    """P(|s_t - μ| ≥ x · σ_∞) ≤ 2 · exp(-x²/2) for stationary OU.

    Returns the upper bound on the two-sided tail probability."""
    return 2.0 * math.exp(-(x_sigmas ** 2) / 2.0)


def ou_tail_quantile(stationary_std: float, alpha: float) -> float:
    """Inverse of the sub-Gaussian tail bound: returns x such that the bound
    P(|s_t - μ| ≥ x · σ_∞) ≤ α.

    Solving 2 exp(-x²/2) = α gives x = √(2 ln(2/α))."""
    if alpha <= 0 or alpha >= 2:
        return float("inf")
    return stationary_std * math.sqrt(2.0 * math.log(2.0 / alpha))


# ===========================================================================
# Smoke test
# ===========================================================================

if __name__ == "__main__":
    import random

    print("=== §1 conformal prediction coverage test ===")
    random.seed(0)
    # Simulate 1000 calibration residuals from a t-distribution with df=4 (heavy tail)
    def t4_sample():
        u = sum(random.gauss(0, 1) ** 2 for _ in range(4)) / 4
        return random.gauss(0, 1) / math.sqrt(u)
    cal_resid = [t4_sample() for _ in range(500)]
    # Test coverage on 1000 holdout points
    coverage = 0
    for _ in range(1000):
        true_y = random.gauss(0, 1)
        pred = 0.0
        interval = conformal_interval(pred, cal_resid, alpha=0.10)
        if interval.lower <= true_y <= interval.upper:
            coverage += 1
    rate = coverage / 1000
    print(f"  target coverage: 90%, realized: {rate*100:.1f}%  → {'PASS' if 0.85 <= rate <= 0.95 else 'CHECK'}")
    print()

    print("=== §2 empirical Bernstein vs CLT ===")
    random.seed(1)
    sample = [random.gauss(0.05, 0.2) for _ in range(720)]
    mean_emp = statistics.mean(sample)
    se_clt = statistics.stdev(sample) / math.sqrt(720)
    z_clt = mean_emp / se_clt
    lo, hi, rad = empirical_bernstein_credibility(sample, delta=1e-6)
    print(f"  sample mean: {mean_emp:.4f}")
    print(f"  asymptotic SE (CLT): {se_clt:.4f}, z = {z_clt:.2f}")
    print(f"  Maurer-Pontil radius (δ=1e-6): {rad:.4f}")
    print(f"  Bernstein interval: [{lo:.4f}, {hi:.4f}]")
    print(f"  Bernstein conservative? {rad > se_clt * 4.7}  (5σ asymptotic ≈ 4.75 SE)")
    print()

    print("=== §3 DRO ε from sample size ===")
    for n in [100, 720, 5000]:
        eps = dro_epsilon_from_sample(n, confidence=0.95, diameter_estimate=0.10)
        print(f"  n={n}: ε = {eps:.5f}")
    print()

    print("=== §4 Hurst recovery ===")
    # H = 0.5 random walk
    rw = [random.gauss(0, 1) for _ in range(720)]
    h_rw = hurst_dfa(rw)
    print(f"  random walk increments (true H=0.5): estimated H = {h_rw:.3f}")
    # Anti-persistent (mean-reverting) sample via OU-ish
    ar = [0.0]
    for _ in range(719):
        ar.append(0.5 * ar[-1] + random.gauss(0, 1))
    h_ar = hurst_dfa(ar)
    print(f"  AR(1) ρ=0.5 (anti-persistent): estimated H = {h_ar:.3f}")
    print()

    print("=== §5 Hawkes fit ===")
    # Generate exponential-kernel Hawkes events
    random.seed(2)
    true_mu0 = 0.5
    true_alpha = 0.6
    true_beta = 1.0
    events = []
    t = 0.0
    T_horizon = 100.0
    while t < T_horizon:
        # Compute current intensity
        intensity = true_mu0 + sum(true_alpha * true_beta * math.exp(-true_beta * (t - tk))
                                    for tk in events)
        u = random.random()
        dt = -math.log(u) / max(intensity, 0.001)
        t += dt
        if t >= T_horizon:
            break
        # Accept with thinning
        new_intensity = true_mu0 + sum(true_alpha * true_beta * math.exp(-true_beta * (t - tk))
                                        for tk in events)
        if random.random() < new_intensity / max(intensity, 0.001):
            events.append(t)
    fit = fit_hawkes(events, T_horizon)
    if fit:
        print(f"  true:      μ_0={true_mu0:.3f}  α={true_alpha:.3f}  β={true_beta:.3f}")
        print(f"  estimated: μ_0={fit.mu_0:.3f}  α={fit.alpha:.3f}  β={fit.beta:.3f}")
        print(f"  n_events={fit.n_events}  expected cluster size = {expected_cluster_size(fit.alpha):.2f}")
    print()

    print("=== §6 Dol Theorem (sub-Gaussian OU tail) ===")
    for x in [1, 2, 3, 4, 5]:
        bound = ou_subgaussian_tail_bound(1.0, x)
        print(f"  P(|s-μ| ≥ {x}σ) ≤ {bound:.6f}")
    print(f"  Inverse (α=0.05): x = {ou_tail_quantile(1.0, 0.05):.3f} σ")
    print(f"  Inverse (α=0.01): x = {ou_tail_quantile(1.0, 0.01):.3f} σ")
    print(f"  Inverse (α=0.001): x = {ou_tail_quantile(1.0, 0.001):.3f} σ")
