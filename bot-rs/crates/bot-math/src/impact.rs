//! D.3 — Effective spread with orderbook impact and competition discount.
//!
//! v4 Part 1.4 final formula:
//!   D̄(τ; D₀, n) = (1 − θ^impact · n / Π) / (1 + ρ^comp)
//!                × [μ̃ + (D₀ − μ̃) · φ(θ^OU · τ)]
//!
//! Note: n is per-leg notional (not per-pair margin).

use crate::ou::ou_time_averaged_spread;
use bot_types::{AnnualizedRate, Dimensionless, FrameworkError, HourlyRate, Hours, Usd};

/// Effective spread after orderbook impact and mean-field competition discount.
///
/// # Arguments
/// - `d0`            — current observed spread (annualized)
/// - `mu`            — OU long-run mean (annualized)
/// - `theta_ou`      — OU mean-reversion speed (per hour)
/// - `tau_h`         — planning hold horizon (hours)
/// - `n_per_leg`     — per-leg notional in USD (**not** per-pair margin)
/// - `pi_pac`        — Pacifica open interest in USD (Π^pac)
/// - `theta_impact`  — orderbook impact coefficient θ^impact ∈ [0.3, 0.7]
/// - `rho_comp`      — competitor density ρ^comp ≥ 0
///
/// # Returns
/// Effective annualized spread, floored at 0 if impact is total.
// All 8 parameters are independent physics inputs required by the v4 MFG formula.
// Bundling them into a struct would break Python-parity argument ordering.
#[allow(clippy::too_many_arguments)]
pub fn effective_spread_with_impact(
    d0: AnnualizedRate,
    mu: AnnualizedRate,
    theta_ou: HourlyRate,
    tau_h: Hours,
    n_per_leg: Usd,
    pi_pac: Usd,
    theta_impact: Dimensionless,
    rho_comp: Dimensionless,
) -> Result<AnnualizedRate, FrameworkError> {
    if pi_pac.0 <= 0.0 {
        return Err(FrameworkError::InvalidInput(
            "Π^pac must be positive".into(),
        ));
    }
    if rho_comp.0 < 0.0 {
        return Err(FrameworkError::InvalidInput(
            "ρ^comp must be non-negative".into(),
        ));
    }

    let ou_avg = ou_time_averaged_spread(d0, mu, theta_ou, tau_h);
    let impact_factor = 1.0 - theta_impact.0 * n_per_leg.0 / pi_pac.0;
    let comp_factor = 1.0 / (1.0 + rho_comp.0);

    // If impact_factor ≤ 0 the position is too large: signal fully destroyed.
    if impact_factor <= 0.0 {
        return Ok(AnnualizedRate(0.0));
    }

    Ok(AnnualizedRate(ou_avg.0 * impact_factor * comp_factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_impact_zero_competition_equals_ou_avg() {
        let d0 = AnnualizedRate(0.12);
        let mu = AnnualizedRate(0.10);
        let theta = HourlyRate(0.01);
        let tau = Hours(100.0);

        let ou = ou_time_averaged_spread(d0, mu, theta, tau);
        let eff = effective_spread_with_impact(
            d0,
            mu,
            theta,
            tau,
            Usd(0.0), // zero notional → zero impact
            Usd(1_000_000.0),
            Dimensionless(0.5),
            Dimensionless(0.0),
        )
        .unwrap();

        assert!((eff.0 - ou.0).abs() < 1e-15);
    }

    #[test]
    fn large_notional_kills_signal() {
        // n_per_leg = π → impact_factor = 1 - 0.5*1 = 0.5 → not zero
        // n_per_leg = 2*π/θ → impact_factor = 0 → clamp to 0
        let result = effective_spread_with_impact(
            AnnualizedRate(0.10),
            AnnualizedRate(0.10),
            HourlyRate(0.01),
            Hours(100.0),
            Usd(3_000_000.0), // huge notional
            Usd(1_000_000.0), // smaller OI
            Dimensionless(0.5),
            Dimensionless(0.0),
        )
        .unwrap();

        assert_eq!(result.0, 0.0);
    }

    #[test]
    fn negative_pi_returns_error() {
        let err = effective_spread_with_impact(
            AnnualizedRate(0.10),
            AnnualizedRate(0.10),
            HourlyRate(0.01),
            Hours(100.0),
            Usd(1000.0),
            Usd(-1.0),
            Dimensionless(0.5),
            Dimensionless(0.0),
        );
        assert!(matches!(err, Err(FrameworkError::InvalidInput(_))));
    }

    #[test]
    fn negative_rho_returns_error() {
        let err = effective_spread_with_impact(
            AnnualizedRate(0.10),
            AnnualizedRate(0.10),
            HourlyRate(0.01),
            Hours(100.0),
            Usd(1000.0),
            Usd(1_000_000.0),
            Dimensionless(0.5),
            Dimensionless(-0.1),
        );
        assert!(matches!(err, Err(FrameworkError::InvalidInput(_))));
    }

    #[test]
    fn competition_halves_spread() {
        let d0 = AnnualizedRate(0.10);
        let mu = AnnualizedRate(0.10);
        let theta = HourlyRate(0.01);
        let tau = Hours(168.0);

        // No competition
        let base = effective_spread_with_impact(
            d0,
            mu,
            theta,
            tau,
            Usd(0.0),
            Usd(1_000_000.0),
            Dimensionless(0.5),
            Dimensionless(0.0),
        )
        .unwrap();

        // ρ = 1 → comp_factor = 0.5
        let halved = effective_spread_with_impact(
            d0,
            mu,
            theta,
            tau,
            Usd(0.0),
            Usd(1_000_000.0),
            Dimensionless(0.5),
            Dimensionless(1.0),
        )
        .unwrap();

        assert!((halved.0 - base.0 / 2.0).abs() < 1e-15);
    }
}
