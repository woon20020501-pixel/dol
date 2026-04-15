//! D.6–D.7 — Critical AUM and Bernstein robust leverage bound.
//!
//! D.6 v4 Part 4.4: critical AUM A^crit
//! D.7 v4 Part 5.3: Bernstein concentration-inequality leverage bound L^R

use bot_types::{AumFraction, Dimensionless, FrameworkError, Hours, Usd};

/// Critical AUM A^crit_i (regime boundary between leverage-binding and decay-binding).
///
/// # Formula
/// A^crit_i = Π_i · (1 − τ^BE / τ) / (θ^impact · L · m_pos)
///
/// # Regime interpretation
/// - A < A^crit : w_i = m_pos is binding (leverage-binding)
/// - A > A^crit : w_i = w*_i (decay-binding, interior optimum active)
///
/// Returns `Usd(f64::INFINITY)` if there is no edge (factor ≤ 0) or m_pos ≤ 0.
pub fn critical_aum(
    pi_pac: Usd,
    tau_be: Hours,
    tau: Hours,
    theta_impact: Dimensionless,
    leverage: u32,
    m_pos: AumFraction,
) -> Usd {
    let factor = 1.0 - tau_be.0 / tau.0;
    if factor <= 0.0 || m_pos.0 <= 0.0 {
        return Usd(f64::INFINITY);
    }
    Usd(pi_pac.0 * factor / (theta_impact.0 * leverage as f64 * m_pos.0))
}

/// Bernstein concentration-inequality robust leverage bound L^R(τ).
///
/// # Formula
/// L^R(τ) = [MMR + Δ·L_ε/3 + √((Δ·L_ε/3)² + 2·τ·σ²·L_ε)]^(−1)
///
/// where L_ε = ln(1/ε) and τ is in hours (variance accumulates linearly in time).
///
/// # Arguments
/// - `mmr`          — maintenance margin rate (e.g. 0.03 for 3%)
/// - `delta_per_h`  — per-hour hard bound on mark price move
/// - `sigma_per_h`  — per-hour std of mark price move
/// - `tau_h`        — hold horizon (hours)
/// - `epsilon`      — target liquidation probability ∈ (0, 1)
///
/// # Returns
/// Integer leverage floor (minimum 1). Returns `Err(InvalidInput)` for ε ∉ (0, 1).
pub fn bernstein_leverage_bound(
    mmr: Dimensionless,
    delta_per_h: Dimensionless,
    sigma_per_h: Dimensionless,
    tau_h: Hours,
    epsilon: f64,
) -> Result<u32, FrameworkError> {
    if epsilon <= 0.0 || epsilon >= 1.0 {
        return Err(FrameworkError::InvalidInput(
            "epsilon must be in (0, 1)".into(),
        ));
    }
    let l_eps = (1.0 / epsilon).ln();
    let delta_term = delta_per_h.0 * l_eps / 3.0;
    // Variance term: σ² × τ (τ in hours, consistent with σ in per-√hour)
    let var_term = 2.0 * tau_h.0 * sigma_per_h.0 * sigma_per_h.0 * l_eps;
    let sqrt_term = (delta_term * delta_term + var_term).sqrt();
    let y_star = delta_term + sqrt_term;
    let l_cont = 1.0 / (mmr.0 + y_star);

    if l_cont < 1.0 {
        Ok(1)
    } else {
        Ok(l_cont.floor() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_aum_basic() {
        // Π = 1e7, factor = 0.5, θ = 0.5, L = 3, m_pos = 0.02
        // A^crit = 1e7 * 0.5 / (0.5 * 3 * 0.02) = 5e6 / 0.03 ≈ 166_666.67
        let a = critical_aum(
            Usd(10_000_000.0),
            Hours(50.0),
            Hours(100.0),
            Dimensionless(0.5),
            3,
            AumFraction(0.02),
        );
        let expected = 10_000_000.0 * 0.5 / (0.5 * 3.0 * 0.02);
        assert!((a.0 - expected).abs() < 1e-4);
    }

    #[test]
    fn critical_aum_no_edge_returns_infinity() {
        let a = critical_aum(
            Usd(10_000_000.0),
            Hours(200.0), // τ^BE > τ
            Hours(100.0),
            Dimensionless(0.5),
            3,
            AumFraction(0.02),
        );
        assert!(a.0.is_infinite());
    }

    #[test]
    fn critical_aum_zero_mpos_returns_infinity() {
        let a = critical_aum(
            Usd(10_000_000.0),
            Hours(50.0),
            Hours(100.0),
            Dimensionless(0.5),
            3,
            AumFraction(0.0),
        );
        assert!(a.0.is_infinite());
    }

    #[test]
    fn bernstein_leverage_epsilon_out_of_range() {
        assert!(matches!(
            bernstein_leverage_bound(
                Dimensionless(0.03),
                Dimensionless(0.001),
                Dimensionless(0.005),
                Hours(168.0),
                0.0,
            ),
            Err(FrameworkError::InvalidInput(_))
        ));
        assert!(matches!(
            bernstein_leverage_bound(
                Dimensionless(0.03),
                Dimensionless(0.001),
                Dimensionless(0.005),
                Hours(168.0),
                1.0,
            ),
            Err(FrameworkError::InvalidInput(_))
        ));
    }

    #[test]
    fn bernstein_leverage_returns_at_least_one() {
        // With very high volatility, bound should still be at least 1.
        let l = bernstein_leverage_bound(
            Dimensionless(0.50), // 50% MMR — very conservative
            Dimensionless(0.10), // large delta
            Dimensionless(0.10), // large sigma
            Hours(720.0),
            0.05,
        )
        .unwrap();
        assert!(l >= 1);
    }

    #[test]
    fn bernstein_leverage_decreases_with_longer_horizon() {
        // Longer hold → more price-path variance → tighter leverage bound.
        let l_short = bernstein_leverage_bound(
            Dimensionless(0.03),
            Dimensionless(0.001),
            Dimensionless(0.003),
            Hours(24.0),
            0.01,
        )
        .unwrap();
        let l_long = bernstein_leverage_bound(
            Dimensionless(0.03),
            Dimensionless(0.001),
            Dimensionless(0.003),
            Hours(720.0),
            0.01,
        )
        .unwrap();
        assert!(l_long <= l_short);
    }
}
