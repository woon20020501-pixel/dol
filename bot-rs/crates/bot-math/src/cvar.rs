//! Conditional Value-at-Risk (CVaR, aka Expected Shortfall, aka ES).
//!
//! Reference: Rockafellar & Uryasev (2000), "Optimization of conditional
//! value-at-risk", Journal of Risk 2(3):21-41.
//!
//! Formal definition (continuous distribution F_L of loss L):
//!
//! ```text
//!     CVaR_α(L) = E[L | L ≥ VaR_α(L)]
//! ```
//!
//! For empirical samples this is the mean of the upper (1-α) tail.
//!
//! The variational formulation is:
//!
//! ```text
//!     CVaR_α(L) = min_ξ { ξ + (1-α)^(-1) · E[(L-ξ)^+] }
//! ```
//!
//! and the minimizer ξ* equals VaR_α(L). For empirical L with n samples
//! the closed-form sample CVaR reduces to:
//!
//! ```text
//!     Sort losses descending: L_(1) ≥ L_(2) ≥ ... ≥ L_(n)
//!     k = ⌈(1-α) · n⌉
//!     CVaR_α ≈ (1/k) · Σ_{i=1..k} L_(i)
//! ```
//!
//! This is the standard empirical estimator and matches Python
//! `risk_stack.cvar_ru` as of aurora-omega-1.1.3.

/// Empirical CVaR at level α on loss samples.
///
/// `losses` is a slice of realized losses (positive = loss, negative = gain).
/// α is the confidence level, e.g. 0.99 for CVaR_99.
///
/// Returns:
/// - `Some(cvar)` when there are enough samples to form a non-empty tail
///   (i.e. `(1-α)·n ≥ 1`).
/// - `None` when the tail has fewer than one sample (rare — requires
///   `n < 1/(1-α)`, e.g. n < 100 for α = 0.99).
///
/// # Numerical properties
/// - Sorts a local copy of `losses` (O(n log n)); does not mutate caller's slice.
/// - Uses `partial_cmp` with NaN-sinking; NaN losses are treated as -∞ and
///   therefore filtered OUT of the tail. This is the conservative choice:
///   a NaN P&L observation must not inflate the tail toward the cap.
///
/// # Panics
/// None. `α` outside `(0, 1)` returns `None` instead of panicking so callers
/// on the hot path never branch on Result.
pub fn cvar_empirical(losses: &[f64], alpha: f64) -> Option<f64> {
    if !(0.0 < alpha && alpha < 1.0) {
        return None;
    }
    // Filter non-finite losses (NaN, ±Inf) BEFORE sizing the tail so that
    // a single NaN observation can't inflate tail_k past the real sample
    // count and trigger the "not enough data" branch unnecessarily.
    let mut sorted: Vec<f64> = losses.iter().copied().filter(|x| x.is_finite()).collect();
    let n_eff = sorted.len();
    if n_eff == 0 {
        return None;
    }
    let tail_k = ((1.0 - alpha) * n_eff as f64).ceil() as usize;
    if tail_k == 0 || tail_k > n_eff {
        return None;
    }
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let sum: f64 = sorted[..tail_k].iter().sum();
    Some(sum / tail_k as f64)
}

/// Rockafellar-Uryasev variational CVaR with explicit ξ minimization.
///
/// Returns `(cvar, xi_star)` where `xi_star` is the VaR at level α.
/// Used when the caller also needs the VaR quantile (for ξ-based gates).
///
/// Implementation: because the variational form's minimizer is exactly the
/// empirical α-quantile of L, we compute VaR_α directly and derive CVaR
/// as the conditional mean above it. This avoids numerical optimization.
///
/// Formulaic equivalence (proof in Rockafellar-Uryasev Theorem 1):
///
/// ```text
///     VaR_α = F_L^(-1)(α) = L_(⌈α·n⌉)  (ascending order)
///     CVaR_α = VaR_α + (1-α)^(-1) · (1/n) · Σ (L_i - VaR_α)^+
/// ```
pub fn cvar_ru(losses: &[f64], alpha: f64) -> Option<(f64, f64)> {
    if !(0.0 < alpha && alpha < 1.0) {
        return None;
    }
    let finite: Vec<f64> = losses.iter().copied().filter(|x| x.is_finite()).collect();
    let n = finite.len();
    if n == 0 {
        return None;
    }
    let mut ascending = finite.clone();
    ascending.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // VaR_α: ⌈α·n⌉-th order statistic (Python `math.ceil`).
    let k = ((alpha * n as f64).ceil() as usize).max(1);
    let var = ascending[(k - 1).min(n - 1)];

    // CVaR_α = VaR + (1-α)^(-1) · mean((L - VaR)^+)
    let excess_mean: f64 = finite.iter().map(|&l| (l - var).max(0.0)).sum::<f64>() / n as f64;
    let cvar = var + excess_mean / (1.0 - alpha);
    Some((cvar, var))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uniform losses: CVaR_α of uniform(0,1) is (1+α)/2.
    /// For α=0.95 → expected 0.975; tolerance loose because we use finite sample.
    #[test]
    fn cvar_uniform_large_sample_converges() {
        // 10_000 equally-spaced values in (0, 1)
        let samples: Vec<f64> = (1..=10_000).map(|i| i as f64 / 10_000.0).collect();
        let cvar = cvar_empirical(&samples, 0.95).unwrap();
        // Theoretical (1+0.95)/2 = 0.975
        assert!(
            (cvar - 0.975).abs() < 0.005,
            "CVaR of uniform should be ~0.975, got {}",
            cvar
        );
    }

    /// Degenerate constant loss: CVaR = L.
    #[test]
    fn cvar_constant() {
        let samples = vec![5.0; 1_000];
        let cvar = cvar_empirical(&samples, 0.99).unwrap();
        assert!((cvar - 5.0).abs() < 1e-12);
    }

    /// Single sample: tail_k = ceil(0.01 · 1) = 1, CVaR = that sample.
    #[test]
    fn cvar_single_sample() {
        let cvar = cvar_empirical(&[42.0], 0.99).unwrap();
        assert_eq!(cvar, 42.0);
    }

    /// Empty slice → None.
    #[test]
    fn cvar_empty_returns_none() {
        assert!(cvar_empirical(&[], 0.99).is_none());
    }

    /// α out of (0, 1) → None (not a panic).
    #[test]
    fn cvar_alpha_out_of_range() {
        assert!(cvar_empirical(&[1.0, 2.0, 3.0], 0.0).is_none());
        assert!(cvar_empirical(&[1.0, 2.0, 3.0], 1.0).is_none());
        assert!(cvar_empirical(&[1.0, 2.0, 3.0], -0.1).is_none());
        assert!(cvar_empirical(&[1.0, 2.0, 3.0], 1.1).is_none());
    }

    /// NaN losses are excluded; CVaR of remaining finite samples is returned.
    #[test]
    fn cvar_nan_excluded() {
        let samples = vec![1.0, f64::NAN, 2.0, 3.0, f64::INFINITY];
        let cvar = cvar_empirical(&samples, 0.5).unwrap();
        // Finite sample: [1, 2, 3]; tail_k = ceil(0.5 · 3) = 2
        // Top 2 (descending): [3, 2] → mean = 2.5
        assert!((cvar - 2.5).abs() < 1e-12);
    }

    /// Variational RU-formula matches the empirical estimator within tol on uniform.
    #[test]
    fn cvar_ru_matches_empirical() {
        let samples: Vec<f64> = (1..=1_000).map(|i| i as f64 / 1_000.0).collect();
        let e = cvar_empirical(&samples, 0.95).unwrap();
        let (ru, _var) = cvar_ru(&samples, 0.95).unwrap();
        // Both estimators converge to the same tail-mean at n=1000.
        assert!(
            (e - ru).abs() < 0.002,
            "empirical={} vs ru={} should match at large n",
            e,
            ru
        );
    }

    /// VaR from `cvar_ru` is the 95th percentile of the sample.
    #[test]
    fn cvar_ru_returns_correct_var() {
        let samples: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let (_cvar, var) = cvar_ru(&samples, 0.95).unwrap();
        // VaR_95 = 95th percentile = L_(95) (1-indexed ascending) = 95.0
        assert_eq!(var, 95.0);
    }

    /// Monotonicity in α: higher α → larger (or equal) CVaR.
    #[test]
    fn cvar_monotonic_in_alpha() {
        let samples: Vec<f64> = (1..=1_000).map(|i| i as f64 / 100.0).collect();
        let c50 = cvar_empirical(&samples, 0.50).unwrap();
        let c90 = cvar_empirical(&samples, 0.90).unwrap();
        let c99 = cvar_empirical(&samples, 0.99).unwrap();
        assert!(c50 < c90);
        assert!(c90 < c99);
    }
}
