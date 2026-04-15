//! D.1 — Absorption function φ(x) = (1 - e^(−x)) / x.
//!
//! φ(0) := 1  (L'Hôpital / Taylor limit).
//! v4 Part 1.4: OU mean-reversion absorption factor.
//!
//! Numerical stability: the naive `(1 - e^(-x))/x` form suffers catastrophic
//! cancellation for |x| ≲ 1e-8. We use `expm1` for the main branch and a
//! Taylor expansion below the threshold. Matches the Python reference
//! `phi_reference` in `strategy/scripts/generate_rust_fixtures.py`.
//! Branch thresholds (1e-8 for φ, 1e-4 for φ′) are policy — do NOT retune.

/// φ(x) = (1 − e^(−x)) / x, with φ(0) := 1.
///
/// Uses `f64::exp_m1` (libm `expm1`) for numerical stability. For |x| < 1e-8
/// falls back to a 4-term Taylor expansion. Matches Python `phi_reference`.
///
/// Boundary values:
///   φ(0)   = 1
///   φ(1)   ≈ 0.6321205588
///   φ(10)  ≈ 0.09999546
///   φ(100) < 0.011
///   Monotone decreasing for x > 0.
pub fn phi(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() < 1e-8 {
        // Taylor: 1 − x/2 + x²/6 − x³/24 + O(x⁴)
        return 1.0 - x / 2.0 + x * x / 6.0 - x * x * x / 24.0;
    }
    // (1 − e^(−x)) / x == −expm1(−x) / x  (stable near zero)
    -(-x).exp_m1() / x
}

/// φ′(x) = [(1 + x) e^(−x) − 1] / x².
///
/// Uses a 4-term Taylor expansion for |x| < 1e-4 to avoid cancellation in the
/// numerator. Always negative for x > 0 (φ is strictly decreasing).
/// Matches Python `phi_derivative_reference`.
pub fn phi_derivative(x: f64) -> f64 {
    if x == 0.0 {
        return -0.5;
    }
    if x.abs() < 1e-4 {
        // Taylor: −1/2 + x/3 − x²/8 + x³/30 − O(x⁴)
        return -0.5 + x / 3.0 - x * x / 8.0 + x * x * x / 30.0;
    }
    ((1.0 + x) * (-x).exp() - 1.0) / (x * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Boundary-value sanity tests
    // -----------------------------------------------------------------------

    #[test]
    fn phi_at_zero() {
        assert!((phi(0.0) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn phi_at_one() {
        // (1 - e^-1) / 1 = 1 - 1/e
        let expected = 1.0 - (-1.0_f64).exp();
        assert!((phi(1.0) - expected).abs() < 1e-15);
        // Documented value
        assert!((phi(1.0) - 0.6321205588).abs() < 1e-8);
    }

    #[test]
    fn phi_at_ten() {
        assert!((phi(10.0) - 0.09999546).abs() < 1e-6);
    }

    #[test]
    fn phi_at_large() {
        assert!(phi(100.0) < 0.011);
    }

    #[test]
    fn phi_monotone_decreasing() {
        let mut prev = phi(0.0);
        for i in 1..=100 {
            let x = i as f64 * 0.1;
            let curr = phi(x);
            assert!(curr < prev, "phi not monotone decreasing at x={x}");
            prev = curr;
        }
    }

    #[test]
    fn phi_derivative_always_negative_for_positive_x() {
        for i in 1..=100 {
            let x = i as f64 * 0.1;
            assert!(phi_derivative(x) < 0.0, "phi_derivative ≥ 0 at x={x}");
        }
    }

    /// Verify derivative numerically at a few points using central differences.
    #[test]
    fn phi_derivative_numerical_consistency() {
        let h = 1e-7;
        for &x in &[0.5, 1.0, 2.0, 5.0, 10.0] {
            let numeric = (phi(x + h) - phi(x - h)) / (2.0 * h);
            let analytic = phi_derivative(x);
            assert!(
                (numeric - analytic).abs() < 1e-8,
                "phi_derivative mismatch at x={x}: numeric={numeric}, analytic={analytic}"
            );
        }
    }

    /// At x just above the Taylor threshold (1e-8) the expm1-based general
    /// branch must agree with the Taylor expansion to f64 precision.
    #[test]
    fn phi_near_zero_continuity() {
        let eps = 1e-7; // above 1e-8 threshold
        let actual = phi(eps);
        let taylor_ref = 1.0 - eps / 2.0 + eps * eps / 6.0 - eps * eps * eps / 24.0;
        assert!(
            (actual - taylor_ref).abs() < 1e-15,
            "phi continuity at eps={eps}: actual={actual}, taylor_ref={taylor_ref}"
        );
    }

    /// phi(0) via Taylor equals exactly 1.0.
    #[test]
    fn phi_zero_exact() {
        // x = 0 goes into Taylor branch: 1 - 0/2 + 0/6 = 1
        assert_eq!(phi(0.0), 1.0 - 0.0 / 2.0 + 0.0_f64 * 0.0 / 6.0);
    }
}
