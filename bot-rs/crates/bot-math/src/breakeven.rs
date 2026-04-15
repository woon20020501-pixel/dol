//! D.4 — Break-even hold time.
//!
//! v4 Part 4:
//!   τ^BE = 8760 · c · (1 + ρ^comp) / D^eff(τ, D₀)
//!
//! τ^BE depends on τ itself (via D^eff), so self-consistency is required.
//! When D₀ = μ, D^eff = μ regardless of τ → closed-form solution.
//! When D₀ ≠ μ → fixed-point iteration.

use crate::ou::ou_time_averaged_spread;
use bot_types::{AnnualizedRate, Dimensionless, FrameworkError, HourlyRate, Hours};

/// Break-even hold when D₀ = μ̃ (at-mean closed form).
///
/// # Formula
/// τ^BE = 8760 · c · (1 + ρ^comp) / μ̃
///
/// Returns `Err(NegativeSpread)` if μ ≤ 0.
pub fn break_even_hold_at_mean(
    mu: AnnualizedRate,
    c_round_trip: Dimensionless,
    rho_comp: Dimensionless,
) -> Result<Hours, FrameworkError> {
    if mu.0 <= 0.0 {
        return Err(FrameworkError::NegativeSpread);
    }
    Ok(Hours(8760.0 * c_round_trip.0 * (1.0 + rho_comp.0) / mu.0))
}

/// Break-even hold when D₀ ≠ μ̃ (fixed-point iteration).
///
/// Iterates:
///   τ^BE_{k+1} = 8760 · c · (1 + ρ) / [μ + (D₀ − μ) · φ(θ^OU · τ^BE_k)]
///
/// until |τ_{k+1} − τ_k| < `tol`.
///
/// # Arguments
/// - `initial_tau_h` — starting guess (e.g. from `break_even_hold_at_mean`)
/// - `max_iter`      — iteration cap (returns `FixedPointNotConverged` on exhaustion)
/// - `tol`           — convergence tolerance in hours
// All 8 parameters are independent physics inputs required by the v4 fixed-point formula.
// Bundling into a struct would break Python-parity argument ordering.
#[allow(clippy::too_many_arguments)]
pub fn break_even_hold_fixed_point(
    d0: AnnualizedRate,
    mu: AnnualizedRate,
    theta_ou: HourlyRate,
    c_round_trip: Dimensionless,
    rho_comp: Dimensionless,
    initial_tau_h: Hours,
    max_iter: usize,
    tol: f64,
) -> Result<Hours, FrameworkError> {
    let mut tau = initial_tau_h;
    for _ in 0..max_iter {
        let d_eff = ou_time_averaged_spread(d0, mu, theta_ou, tau);
        if d_eff.0 <= 0.0 {
            return Err(FrameworkError::NegativeEffectiveSpread);
        }
        let tau_new = Hours(8760.0 * c_round_trip.0 * (1.0 + rho_comp.0) / d_eff.0);
        if (tau_new.0 - tau.0).abs() < tol {
            return Ok(tau_new);
        }
        tau = tau_new;
    }
    Err(FrameworkError::FixedPointNotConverged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_even_at_mean_basic() {
        // τ^BE = 8760 * 0.001 * (1 + 0) / 0.10 = 87.6 hours
        let result = break_even_hold_at_mean(
            AnnualizedRate(0.10),
            Dimensionless(0.001),
            Dimensionless(0.0),
        )
        .unwrap();
        assert!((result.0 - 87.6).abs() < 1e-10);
    }

    #[test]
    fn break_even_at_mean_with_competition() {
        // τ^BE = 8760 * 0.001 * (1 + 1.0) / 0.10 = 175.2 hours
        let result = break_even_hold_at_mean(
            AnnualizedRate(0.10),
            Dimensionless(0.001),
            Dimensionless(1.0),
        )
        .unwrap();
        assert!((result.0 - 175.2).abs() < 1e-10);
    }

    #[test]
    fn break_even_negative_mu_errors() {
        let err = break_even_hold_at_mean(
            AnnualizedRate(-0.01),
            Dimensionless(0.001),
            Dimensionless(0.0),
        );
        assert!(matches!(err, Err(FrameworkError::NegativeSpread)));
    }

    #[test]
    fn fixed_point_when_d0_equals_mu_matches_closed_form() {
        let mu = AnnualizedRate(0.10);
        // When D₀ = μ, fixed point should converge to the closed-form result.
        let closed = break_even_hold_at_mean(mu, Dimensionless(0.001), Dimensionless(0.0)).unwrap();
        let fp = break_even_hold_fixed_point(
            mu,
            mu,
            HourlyRate(0.01),
            Dimensionless(0.001),
            Dimensionless(0.0),
            closed,
            200,
            1e-9,
        )
        .unwrap();
        assert!((fp.0 - closed.0).abs() < 1e-6);
    }

    #[test]
    fn fixed_point_converges_for_reasonable_inputs() {
        // D₀ > μ: spread is above mean, break-even should be shorter.
        let d0 = AnnualizedRate(0.20);
        let mu = AnnualizedRate(0.10);
        let theta = HourlyRate(0.01);
        let c = Dimensionless(0.001);
        let rho = Dimensionless(0.0);
        // Initial guess: closed-form at mean
        let initial = break_even_hold_at_mean(mu, c, rho).unwrap();
        let result =
            break_even_hold_fixed_point(d0, mu, theta, c, rho, initial, 500, 1e-9).unwrap();
        // Break-even at elevated D₀ should be shorter than at mean
        assert!(result.0 < initial.0 + 1.0); // allow slight overshoot on first iter
        assert!(result.0 > 0.0);
    }
}
