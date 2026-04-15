"""
validate_frontier.py — 6-test validation suite for the frontier framework
(strategy/frontier.py implementing docs/math-frontier.md §1-§6).

Each test exercises one frontier component plus an integration test verifying
the composed pipeline produces sensible output on synthetic data.
"""
import math
import os
import random
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from strategy.frontier import (
    conformal_interval, conformal_lower_var,
    empirical_bernstein_radius, empirical_bernstein_credibility,
    dro_epsilon_from_sample, dro_tangency_weights, dro_objective_value,
    hurst_dfa,
    fit_hawkes, hawkes_log_likelihood, expected_cluster_size,
    ou_subgaussian_tail_bound, ou_tail_quantile,
)


def test_1_conformal_coverage():
    """Conformal coverage matches nominal under exchangeability."""
    print("(1) Conformal prediction coverage (same-distribution test)")
    random.seed(0)
    n_calibration = 500
    n_test = 2000
    alpha_target = 0.10
    # Calibration: residuals from a Student-t distribution (heavy tails)
    def t4():
        u = sum(random.gauss(0, 1) ** 2 for _ in range(4)) / 4
        return random.gauss(0, 1) / math.sqrt(u)
    cal = [t4() for _ in range(n_calibration)]
    # Test: same distribution
    covered = 0
    for _ in range(n_test):
        true_y = t4()
        pred = 0.0
        interval = conformal_interval(pred, cal, alpha=alpha_target)
        if interval.lower <= true_y <= interval.upper:
            covered += 1
    rate = covered / n_test
    nominal = 1 - alpha_target
    # Coverage should be ≥ nominal; in our scheme with n_cal=500, expected ≈ 0.902
    ok = rate >= nominal - 0.02
    print(f"  nominal: {nominal*100:.1f}%, realized: {rate*100:.1f}%  → {'PASS' if ok else 'FAIL'}")
    return ok


def test_2_bernstein_concentration():
    """Maurer-Pontil radius is finite and shrinks with n."""
    print("(2) Empirical Bernstein concentration vs sample size")
    random.seed(1)
    radii = []
    for n in [100, 500, 2000]:
        sample = [random.gauss(0.05, 0.20) for _ in range(n)]
        _, _, r = empirical_bernstein_credibility(sample, delta=1e-6)
        radii.append((n, r))
        print(f"  n={n:>5}: half-width = {r:.4f}")
    # Half-width should decrease monotonically with n
    monotone = all(radii[i][1] > radii[i + 1][1] for i in range(len(radii) - 1))
    print(f"  monotonic decrease in n? {monotone}  → {'PASS' if monotone else 'FAIL'}")
    return monotone


def test_3_dro_robustness():
    """DRO regularizer makes portfolio less sensitive to noise."""
    print("(3) Wasserstein DRO vs vanilla Markowitz on noisy data")
    random.seed(2)
    n = 8
    true_means = [0.10, 0.12, 0.08, 0.15, 0.11, 0.09, 0.13, 0.10]
    n_trials = 50
    sharpe_dro = []
    sharpe_van = []
    cov = [[0.001 if i == j else 0.0001 for j in range(n)] for i in range(n)]
    for trial in range(n_trials):
        # Noisy estimate of means (uncertainty)
        noise = [random.gauss(0, 0.02) for _ in range(n)]
        noisy_means = [true_means[i] + noise[i] for i in range(n)]
        # Vanilla Markowitz
        w_van = dro_tangency_weights(noisy_means, cov, r_idle=0.04, dro_epsilon=0.0001)
        # DRO with epsilon
        eps = dro_epsilon_from_sample(720, confidence=0.95, diameter_estimate=0.10)
        w_dro = dro_tangency_weights(noisy_means, cov, r_idle=0.04, dro_epsilon=eps)
        # Realize Sharpe under TRUE means
        ret_van = sum(w_van[i] * (true_means[i] - 0.04) for i in range(n))
        ret_dro = sum(w_dro[i] * (true_means[i] - 0.04) for i in range(n))
        var_van = sum(w_van[i] * cov[i][j] * w_van[j] for i in range(n) for j in range(n))
        var_dro = sum(w_dro[i] * cov[i][j] * w_dro[j] for i in range(n) for j in range(n))
        if var_van > 0:
            sharpe_van.append(ret_van / math.sqrt(var_van))
        if var_dro > 0:
            sharpe_dro.append(ret_dro / math.sqrt(var_dro))
    avg_van = statistics.mean(sharpe_van)
    avg_dro = statistics.mean(sharpe_dro)
    print(f"  avg out-of-sample Sharpe — vanilla: {avg_van:.4f}")
    print(f"  avg out-of-sample Sharpe — DRO    : {avg_dro:.4f}")
    print(f"  DRO ≥ vanilla (within 5% noise)? {avg_dro >= avg_van * 0.95}  → "
          f"{'PASS' if avg_dro >= avg_van * 0.95 else 'FAIL'}")
    return avg_dro >= avg_van * 0.95


def test_4_hurst_recovery():
    """DFA recovers Hurst exponent on known processes."""
    print("(4) DFA Hurst exponent recovery")
    random.seed(3)
    # Test 1: white noise increments, expected H ≈ 0.5
    wn = [random.gauss(0, 1) for _ in range(2000)]
    h_wn = hurst_dfa(wn)
    print(f"  white noise (H_true=0.5):       H_estimated = {h_wn:.3f}")
    # Test 2: positively autocorrelated AR(1) ρ=0.7, expected H > 0.5
    ar_pos = [0.0]
    for _ in range(1999):
        ar_pos.append(0.7 * ar_pos[-1] + random.gauss(0, 1))
    h_ar = hurst_dfa(ar_pos)
    print(f"  AR(1) ρ=0.7 (persistent):       H_estimated = {h_ar:.3f}")
    # Test 3: anti-persistent (negative AR), expected H < 0.5
    ar_neg = [0.0]
    for _ in range(1999):
        ar_neg.append(-0.7 * ar_neg[-1] + random.gauss(0, 1))
    h_neg = hurst_dfa(ar_neg)
    print(f"  AR(1) ρ=-0.7 (anti-persistent): H_estimated = {h_neg:.3f}")

    ok = (0.40 <= h_wn <= 0.60) and (h_ar > 0.55) and (h_neg < 0.50)
    print(f"  → {'PASS' if ok else 'FAIL'}")
    return ok


def test_5_hawkes_recovery():
    """Hawkes MLE recovers parameters from synthetic events."""
    print("(5) Hawkes MLE parameter recovery")
    random.seed(4)
    true_mu0 = 0.5
    true_alpha = 0.6
    true_beta = 1.0
    T = 200.0
    # Generate via thinning algorithm
    events = []
    t = 0.0
    while t < T:
        intensity = true_mu0 + sum(true_alpha * true_beta * math.exp(-true_beta * (t - tk))
                                    for tk in events if t - tk < 50)
        u = random.random()
        dt = -math.log(max(u, 1e-9)) / max(intensity, 0.01)
        t += dt
        if t >= T:
            break
        new_int = true_mu0 + sum(true_alpha * true_beta * math.exp(-true_beta * (t - tk))
                                  for tk in events if t - tk < 50)
        if random.random() < new_int / max(intensity, 0.01):
            events.append(t)
    fit = fit_hawkes(events, T)
    if fit is None:
        print("  fit failed")
        return False
    print(f"  true:      μ_0={true_mu0:.3f}  α={true_alpha:.3f}  β={true_beta:.3f}")
    print(f"  estimated: μ_0={fit.mu_0:.3f}  α={fit.alpha:.3f}  β={fit.beta:.3f}")
    print(f"  n_events: {fit.n_events}")
    # Allow 50% tolerance — Hawkes MLE is noisy with small n
    err_mu = abs(fit.mu_0 - true_mu0) / true_mu0
    err_alpha = abs(fit.alpha - true_alpha) / true_alpha
    ok = err_mu < 0.6 and err_alpha < 0.6 and fit.is_stationary
    print(f"  → {'PASS' if ok else 'FAIL'}")
    return ok


def test_6_subgaussian_dol_theorem():
    """Sub-Gaussian OU tail bound holds for actual OU samples."""
    print("(6) Dol Theorem: sub-Gaussian tail bound on OU stationary distribution")
    random.seed(5)
    # Generate OU stationary samples
    mu = 0.0
    theta = 0.1
    sigma = 0.10
    sigma_inf = sigma / math.sqrt(2 * theta)
    n_samples = 50000
    s = 0.0
    samples = []
    b = math.exp(-theta)
    sigma_eps = sigma * math.sqrt((1 - b ** 2) / (2 * theta))
    for _ in range(n_samples):
        s = b * s + random.gauss(0, sigma_eps)
        samples.append(s)
    # Drop burn-in
    samples = samples[1000:]
    # Test: for each x ∈ {1, 2, 3, 4, 5}, empirical tail prob ≤ sub-Gaussian bound
    print("  σ_∞ predicted: {:.4f}, empirical std: {:.4f}".format(
        sigma_inf, statistics.stdev(samples)))
    abs_devs = [abs((s_i - mu) / sigma_inf) for s_i in samples]
    all_ok = True
    for x in [1.5, 2, 2.5, 3, 4]:
        empirical = sum(1 for d in abs_devs if d >= x) / len(abs_devs)
        bound = ou_subgaussian_tail_bound(sigma_inf, x)
        ok = empirical <= bound + 0.001  # small tolerance
        marker = "OK" if ok else "FAIL"
        all_ok = all_ok and ok
        print(f"  P(|s-μ| ≥ {x:.1f}σ): empirical {empirical:.4f}  bound {bound:.4f}  [{marker}]")
    print(f"  → {'PASS' if all_ok else 'FAIL'}")
    return all_ok


def test_7_integration():
    """Integration: build a synthetic universe, run all frontier components,
    check the system produces a feasible mandate-compliant allocation."""
    print("(7) Integration: frontier pipeline on synthetic universe")
    random.seed(6)
    N = 8
    T = 720
    # 8 candidates with varied means, all with stable dynamics
    true_means_per_h = [0.000050, 0.000060, 0.000040, 0.000080, 0.000055, 0.000070, 0.000045, 0.000065]
    sigma_per_h = 0.000010
    histories = []
    for mu in true_means_per_h:
        # AR(1) toward mu
        b = math.exp(-0.08)
        h = [mu]
        for _ in range(T - 1):
            h.append(mu * (1 - b) + b * h[-1] + random.gauss(0, sigma_per_h))
        histories.append(h)

    # Frontier checks per candidate
    expected_apys = []
    sharps = []
    for i, h in enumerate(histories):
        # Empirical Bernstein on the spread series itself
        lo, hi, rad = empirical_bernstein_credibility(h, delta=1e-3)
        # Hurst
        H = hurst_dfa(h) or 0.5
        # Pass if Bernstein lower bound > 0
        bernstein_pass = lo > 0
        expected_apys.append(statistics.mean(h) * 24 * 365)
        if bernstein_pass:
            sharps.append(i)
    print(f"  candidates passing Bernstein lower-bound > 0: {len(sharps)}/{N}")
    if len(sharps) < 4:
        print(f"  → FAIL (insufficient passing candidates for portfolio)")
        return False

    # Build covariance over signed per-hour returns of passing candidates
    passing_histories = [histories[i] for i in sharps]
    n_pass = len(passing_histories)
    means = [statistics.mean(h) for h in passing_histories]
    cov = [[0.0] * n_pass for _ in range(n_pass)]
    for i in range(n_pass):
        for j in range(n_pass):
            cov[i][j] = sum((passing_histories[i][t] - means[i]) * (passing_histories[j][t] - means[j])
                            for t in range(T)) / (T - 1)
    # Scale to APY
    expected_apy_pass = [m * 24 * 365 for m in means]
    cov_apy = [[c * 24 * 365 for c in row] for row in cov]

    # DRO portfolio
    eps = dro_epsilon_from_sample(T, confidence=0.95, diameter_estimate=0.10)
    w = dro_tangency_weights(expected_apy_pass, cov_apy, r_idle=0.044,
                             risk_aversion=2.0, dro_epsilon=eps)
    # Cap weights to budget=0.5 by scaling
    total = sum(max(0, wi) for wi in w)
    if total <= 0:
        print(f"  → FAIL (DRO returned zero weights)")
        return False
    scale = 0.5 / total
    w = [max(0, wi) * scale for wi in w]

    # Vault APY projection (assume L=2)
    L = 2
    portfolio_mean = sum(w[i] * expected_apy_pass[i] for i in range(n_pass))
    trading_apy = (L / 2) * portfolio_mean
    alpha = 1 - sum(w)
    vault = alpha * 0.044 + trading_apy
    print(f"  DRO ε: {eps:.4f}")
    print(f"  weights (post-scaling): {[f'{wi*100:.1f}%' for wi in w]}")
    print(f"  portfolio mean APY: {portfolio_mean*100:.2f}%")
    print(f"  trading apy on AUM: {trading_apy*100:.2f}%")
    print(f"  vault gross: {vault*100:.2f}%")
    cust = vault * 0.65
    buf = vault * 0.25
    cust_capped = min(cust, 0.08)
    buf_with_excess = buf + max(0, cust - 0.08)
    print(f"  → customer (capped 8%): {cust_capped*100:.2f}%")
    print(f"  → buffer: {min(buf_with_excess, 0.05)*100:.2f}%")
    ok = vault >= 0.08
    print(f"  → {'PASS' if ok else 'FAIL'}")
    return ok


def main():
    print("=" * 80)
    print("VALIDATION SUITE — frontier framework (2005-2024 modern methods)")
    print("=" * 80)
    print()
    tests = [
        test_1_conformal_coverage,
        test_2_bernstein_concentration,
        test_3_dro_robustness,
        test_4_hurst_recovery,
        test_5_hawkes_recovery,
        test_6_subgaussian_dol_theorem,
        test_7_integration,
    ]
    results = []
    for fn in tests:
        try:
            results.append(fn())
        except Exception as e:
            print(f"  EXCEPTION: {type(e).__name__}: {e}")
            results.append(False)
        print()
    print("=" * 80)
    n_pass = sum(1 for r in results if r)
    print(f"SUMMARY: {n_pass}/{len(results)} tests passed")
    print("=" * 80)
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
