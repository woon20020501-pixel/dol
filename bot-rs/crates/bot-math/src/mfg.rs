//! D.8 — MFG free-entry equilibrium.
//!
//! v4 Part 4.4 and 5.3–5.4.

use bot_types::{AnnualizedRate, AumFraction, Dimensionless, FrameworkError, Usd};

/// Free-entry Nash-equilibrium competitor count K*.
///
/// # Formula
/// K* = √(Π · D^eff / (θ^impact · C_op)) − 1
///
/// Floored at 0 (cannot have negative competitors).
pub fn mfg_competitor_count(
    pi_pac: Usd,
    d_eff: AnnualizedRate,
    theta_impact: Dimensionless,
    c_op_marginal: Usd,
) -> Result<f64, FrameworkError> {
    if c_op_marginal.0 <= 0.0 || theta_impact.0 <= 0.0 {
        return Err(FrameworkError::InvalidInput(
            "c_op_marginal and theta_impact must be positive".into(),
        ));
    }
    let ratio = pi_pac.0 * d_eff.0 / (theta_impact.0 * c_op_marginal.0);
    if ratio < 0.0 {
        return Ok(0.0);
    }
    Ok((ratio.sqrt() - 1.0).max(0.0))
}

/// Dol's sustainable flow per pair (cost advantage over marginal competitor).
///
/// # Formula
/// V^Dol_flow = C_op^marginal − C_op^Dol
///
/// Returns `Err(NoSustainableEdge)` if Dol's cost ≥ marginal competitor cost.
pub fn dol_sustainable_flow_per_pair(
    c_op_marginal: Usd,
    c_op_dol: Usd,
) -> Result<Usd, FrameworkError> {
    if c_op_marginal.0 <= c_op_dol.0 {
        return Err(FrameworkError::NoSustainableEdge);
    }
    Ok(Usd(c_op_marginal.0 - c_op_dol.0))
}

/// Capacity ceiling A* for the Dol vault.
///
/// # Formula
/// A* = N · ΔC_op / (R_floor − α_min · r_idle)
///
/// Returns `Err(InfeasibleMandate)` if the denominator ≤ 0.
pub fn capacity_ceiling(
    n_active_pairs: u32,
    delta_c_op: Usd,
    r_floor: AnnualizedRate,
    alpha_min: AumFraction,
    r_idle: AnnualizedRate,
) -> Result<Usd, FrameworkError> {
    let denominator = r_floor.0 - alpha_min.0 * r_idle.0;
    if denominator <= 0.0 {
        return Err(FrameworkError::InfeasibleMandate);
    }
    Ok(Usd(n_active_pairs as f64 * delta_c_op.0 / denominator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mfg_competitor_count_basic() {
        // Π = 1e7, D = 0.10, θ = 0.5, C_op = 50_000
        // ratio = 1e7 * 0.10 / (0.5 * 50_000) = 1e6 / 25_000 = 40
        // K* = √40 - 1 ≈ 5.324
        let k = mfg_competitor_count(
            Usd(10_000_000.0),
            AnnualizedRate(0.10),
            Dimensionless(0.5),
            Usd(50_000.0),
        )
        .unwrap();
        let expected = (40.0_f64).sqrt() - 1.0;
        assert!((k - expected).abs() < 1e-10);
    }

    #[test]
    fn mfg_competitor_count_negative_d_returns_zero() {
        let k = mfg_competitor_count(
            Usd(10_000_000.0),
            AnnualizedRate(-0.01),
            Dimensionless(0.5),
            Usd(50_000.0),
        )
        .unwrap();
        assert_eq!(k, 0.0);
    }

    #[test]
    fn mfg_competitor_count_invalid_inputs() {
        assert!(matches!(
            mfg_competitor_count(
                Usd(10_000_000.0),
                AnnualizedRate(0.10),
                Dimensionless(0.0),
                Usd(50_000.0),
            ),
            Err(FrameworkError::InvalidInput(_))
        ));
    }

    #[test]
    fn dol_sustainable_flow_positive() {
        let flow = dol_sustainable_flow_per_pair(Usd(50_000.0), Usd(20_000.0)).unwrap();
        assert!((flow.0 - 30_000.0).abs() < 1e-6);
    }

    #[test]
    fn dol_sustainable_flow_no_edge() {
        let err = dol_sustainable_flow_per_pair(Usd(20_000.0), Usd(20_000.0));
        assert!(matches!(err, Err(FrameworkError::NoSustainableEdge)));
    }

    #[test]
    fn capacity_ceiling_basic() {
        // N = 10, ΔC = 30_000, R_floor = 0.08, α_min = 0.5, r_idle = 0.04
        // denominator = 0.08 - 0.5 * 0.04 = 0.08 - 0.02 = 0.06
        // A* = 10 * 30_000 / 0.06 = 5_000_000
        let a = capacity_ceiling(
            10,
            Usd(30_000.0),
            AnnualizedRate(0.08),
            AumFraction(0.5),
            AnnualizedRate(0.04),
        )
        .unwrap();
        assert!((a.0 - 5_000_000.0).abs() < 1e-4);
    }

    #[test]
    fn capacity_ceiling_infeasible_mandate() {
        // r_floor ≤ α_min * r_idle → infeasible
        let err = capacity_ceiling(
            10,
            Usd(30_000.0),
            AnnualizedRate(0.02), // floor
            AumFraction(0.5),
            AnnualizedRate(0.05), // r_idle → 0.5 * 0.05 = 0.025 > 0.02
        );
        assert!(matches!(err, Err(FrameworkError::InfeasibleMandate)));
    }
}
