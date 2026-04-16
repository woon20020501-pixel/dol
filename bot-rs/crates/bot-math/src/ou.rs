//! D.2 — OU time-averaged spread.
//!
//! v4 Part 1.4 equation:
//!   D̄(τ; D₀) = μ̃ + (D₀ − μ̃) · φ(θ^OU · τ)

use crate::phi::phi;
use bot_types::{AnnualizedRate, HourlyRate, Hours};

/// Time-averaged OU funding spread over hold horizon τ.
///
/// # Arguments
/// - `d0`       — current observed spread (annualized)
/// - `mu`       — OU long-run mean (annualized)
/// - `theta_ou` — OU mean-reversion speed (per hour)
/// - `tau_h`    — planning hold horizon (hours)
///
/// # Formula
/// D̄(τ; D₀) = μ̃ + (D₀ − μ̃) · φ(θ^OU · τ)
pub fn ou_time_averaged_spread(
    d0: AnnualizedRate,
    mu: AnnualizedRate,
    theta_ou: HourlyRate,
    tau_h: Hours,
) -> AnnualizedRate {
    debug_assert!(
        theta_ou.0 >= 0.0,
        "theta_ou must be non-negative (got {})",
        theta_ou.0
    );
    let x = theta_ou.0 * tau_h.0;
    AnnualizedRate(mu.0 + (d0.0 - mu.0) * phi(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When D₀ = μ, the spread equals μ regardless of τ.
    #[test]
    fn at_mean_is_mu() {
        let mu = AnnualizedRate(0.10);
        let result = ou_time_averaged_spread(mu, mu, HourlyRate(0.01), Hours(100.0));
        assert!((result.0 - mu.0).abs() < 1e-15);
    }

    /// As τ → ∞ (large), D̄ → μ (because φ → 0).
    #[test]
    fn large_tau_approaches_mu() {
        let d0 = AnnualizedRate(0.20);
        let mu = AnnualizedRate(0.10);
        let theta = HourlyRate(0.05);
        // τ = 10000h → x = 500, φ(500) ≈ 0.002/500 ≈ 4e-6
        let result = ou_time_averaged_spread(d0, mu, theta, Hours(10_000.0));
        assert!(
            (result.0 - mu.0).abs() < 1e-3,
            "expected near mu, got {}",
            result.0
        );
    }

    /// At τ = 0, D̄ = D₀ (because φ(0) = 1).
    #[test]
    fn at_zero_tau_is_d0() {
        let d0 = AnnualizedRate(0.15);
        let mu = AnnualizedRate(0.08);
        let result = ou_time_averaged_spread(d0, mu, HourlyRate(0.01), Hours(0.0));
        // φ(0) = 1 → D̄ = μ + (D₀ − μ)·1 = D₀
        assert!((result.0 - d0.0).abs() < 1e-15);
    }

    /// Verify formula numerically at x = 1 (θ·τ = 1).
    #[test]
    fn formula_at_x_eq_one() {
        let d0 = AnnualizedRate(0.20);
        let mu = AnnualizedRate(0.10);
        let theta = HourlyRate(0.1);
        let tau = Hours(10.0); // x = 0.1 * 10 = 1.0
        let result = ou_time_averaged_spread(d0, mu, theta, tau);
        // expected = 0.10 + (0.20 - 0.10) * phi(1.0) = 0.10 + 0.10 * 0.6321...
        let expected = 0.10 + 0.10 * crate::phi::phi(1.0);
        assert!((result.0 - expected).abs() < 1e-15);
    }
}
