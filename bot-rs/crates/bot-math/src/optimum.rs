//! D.5 — Interior optimum: w*_i, n*_i, T*_i.
//!
//! v4 Part 4.1–4.3.

use bot_types::{AnnualizedRate, AumFraction, Dimensionless, FrameworkError, Hours, Usd};

/// Optimal margin fraction per pair (interior solution).
///
/// # Formula
/// w*_i = (1 − τ^BE / τ) / (2 γ_i)
///
/// Returns 0 if τ^BE ≥ τ (no edge) or γ_i ≤ 0 (degenerate).
/// Output is an uncapped `AumFraction`; the caller checks against `m_pos`.
pub fn optimal_margin_fraction(tau_be: Hours, tau: Hours, gamma_i: Dimensionless) -> AumFraction {
    let factor = 1.0 - tau_be.0 / tau.0;
    if factor <= 0.0 || gamma_i.0 <= 0.0 {
        return AumFraction(0.0);
    }
    AumFraction(factor / (2.0 * gamma_i.0))
}

/// Optimal per-leg notional (independent of L and A).
///
/// # Formula
/// n*_i = Π_i · (1 − τ^BE / τ) / (2 · θ^impact)
///
/// Returns 0 if τ^BE ≥ τ or θ^impact ≤ 0.
pub fn optimal_notional(
    pi_pac: Usd,
    tau_be: Hours,
    tau: Hours,
    theta_impact: Dimensionless,
) -> Usd {
    let factor = 1.0 - tau_be.0 / tau.0;
    if factor <= 0.0 || theta_impact.0 <= 0.0 {
        return Usd(0.0);
    }
    Usd(pi_pac.0 * factor / (2.0 * theta_impact.0))
}

/// Maximum trading contribution to vault APY per pair (decay-binding regime).
///
/// # Formula
/// T*_i = D^eff · Π / [4 · (1 + ρ) · θ^impact · A] · (1 − τ^BE / τ)²
///
/// Note: this is independent of L (leverage-binding regime uses a different formula).
///
/// Returns `Err(InvalidInput)` if `aum` ≤ 0 or `theta_impact` ≤ 0.
pub fn optimal_trading_contribution(
    d_eff: AnnualizedRate,
    pi_pac: Usd,
    rho_comp: Dimensionless,
    theta_impact: Dimensionless,
    aum: Usd,
    tau_be: Hours,
    tau: Hours,
) -> Result<AnnualizedRate, FrameworkError> {
    if aum.0 <= 0.0 || theta_impact.0 <= 0.0 {
        return Err(FrameworkError::InvalidInput(
            "aum and theta_impact must be positive".into(),
        ));
    }
    let factor = 1.0 - tau_be.0 / tau.0;
    if factor <= 0.0 {
        return Ok(AnnualizedRate(0.0));
    }
    let numerator = d_eff.0 * pi_pac.0 * factor * factor;
    let denominator = 4.0 * (1.0 + rho_comp.0) * theta_impact.0 * aum.0;
    Ok(AnnualizedRate(numerator / denominator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimal_margin_fraction_basic() {
        // factor = 1 - 50/100 = 0.5; gamma = 1.0 → w* = 0.5/2 = 0.25
        let w = optimal_margin_fraction(Hours(50.0), Hours(100.0), Dimensionless(1.0));
        assert!((w.0 - 0.25).abs() < 1e-15);
    }

    #[test]
    fn optimal_margin_fraction_no_edge() {
        let w = optimal_margin_fraction(Hours(200.0), Hours(100.0), Dimensionless(1.0));
        assert_eq!(w.0, 0.0);
    }

    #[test]
    fn optimal_margin_fraction_degenerate_gamma() {
        let w = optimal_margin_fraction(Hours(50.0), Hours(100.0), Dimensionless(0.0));
        assert_eq!(w.0, 0.0);
    }

    #[test]
    fn optimal_notional_basic() {
        // factor = 1 - 50/100 = 0.5; theta = 0.5; pi = 1_000_000
        // n* = 1_000_000 * 0.5 / (2 * 0.5) = 1_000_000 * 0.5 / 1.0 = 500_000
        let n = optimal_notional(
            Usd(1_000_000.0),
            Hours(50.0),
            Hours(100.0),
            Dimensionless(0.5),
        );
        assert!((n.0 - 500_000.0).abs() < 1e-6);
    }

    #[test]
    fn optimal_notional_no_edge() {
        let n = optimal_notional(
            Usd(1_000_000.0),
            Hours(200.0),
            Hours(100.0),
            Dimensionless(0.5),
        );
        assert_eq!(n.0, 0.0);
    }

    #[test]
    fn optimal_trading_contribution_basic() {
        // factor = 0.5; d_eff = 0.10; pi = 1e6; rho = 0; theta = 0.5; A = 1e7
        // numerator   = 0.10 * 1e6 * 0.25 = 25_000
        // denominator = 4 * 1 * 0.5 * 1e7  = 2e7
        // T* = 25_000 / 2e7 = 0.00125
        let t = optimal_trading_contribution(
            AnnualizedRate(0.10),
            Usd(1_000_000.0),
            Dimensionless(0.0),
            Dimensionless(0.5),
            Usd(10_000_000.0),
            Hours(50.0),
            Hours(100.0),
        )
        .unwrap();
        assert!((t.0 - 0.00125).abs() < 1e-12);
    }

    #[test]
    fn optimal_trading_contribution_invalid_aum() {
        let err = optimal_trading_contribution(
            AnnualizedRate(0.10),
            Usd(1_000_000.0),
            Dimensionless(0.0),
            Dimensionless(0.5),
            Usd(0.0),
            Hours(50.0),
            Hours(100.0),
        );
        assert!(matches!(err, Err(FrameworkError::InvalidInput(_))));
    }
}
