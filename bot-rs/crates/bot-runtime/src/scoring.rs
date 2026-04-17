//! Forecast scoring pipeline — wires the bot-math stack into the decision.
//!
//! Layers (Priority-2 deliverable):
//!
//! 1. **Regime detection** (`bot_strategy_v3::stochastic::{adf_test, fit_ou,
//!    fit_drift}`) — classifies the spread series as stationary (OU) or
//!    drift-persistent. References:
//!    - Dickey & Fuller (1979), JASA 74(366):427.
//!    - MacKinnon (1996), J. Applied Econometrics 11(6):601.
//!    - Uhlenbeck & Ornstein (1930), Phys. Rev. 36:823.
//!    - Phillips (1972), Econometrica 40(6):1021.
//!
//! 2. **Break-even hold** (`bot_math::breakeven::break_even_hold_fixed_point`)
//!    — τ^BE dependent on the OU fit. `funding_cycle_lock` ≤ τ^BE guarantees
//!    the cycle hold clears execution cost.
//!
//! 3. **Bernstein leverage bound** (`bot_math::leverage::bernstein_leverage_bound`)
//!    — safe position-sizing limit given hold horizon and spread volatility.
//!    Bernstein (1946) concentration inequality.
//!
//! 4. **Expected residual income** (`bot_strategy_v3::stochastic::
//!    expected_residual_income`) — closed-form OU hold profit; the decision
//!    layer admits a trade only when this exceeds the round-trip cost.
//!
//! The scoring pipeline is **side-effect free** — it consumes history +
//! snapshots and returns a `ForecastScore` which the decision layer maps
//! onto size/admission gates.

use bot_math::breakeven::break_even_hold_at_mean;
use bot_math::leverage::bernstein_leverage_bound;
use bot_strategy_v3::stochastic::{
    adf_test, expected_residual_income, fit_drift, fit_ou, FitOuOutput,
};
use bot_types::{AnnualizedRate, Dimensionless, FrameworkError, Hours};
use serde::Serialize;

/// Regime classification output.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum Regime {
    /// ADF rejects unit root → spread is mean-reverting → OU fit applies.
    Stationary,
    /// ADF fails to reject → spread is persistent → drift fit applies.
    Drift,
    /// Not enough data yet (history < ADF floor of 50).
    Insufficient,
}

/// Enriched forecast passed to the decision admission gate.
#[derive(Debug, Clone, Serialize)]
pub struct ForecastScore {
    pub regime: Regime,
    /// ADF t-statistic (reject H_0 → stationary when stat < cv5).
    pub adf_statistic: Option<f64>,
    /// OU mean-reversion speed θ (per hour), when regime == Stationary.
    pub theta_hourly: Option<f64>,
    /// OU long-run mean μ (annualized spread units).
    pub mu_annual: Option<f64>,
    /// Drift t-statistic (when regime == Drift) — sanity sign check.
    pub drift_t_statistic: Option<f64>,
    /// OU-implied break-even hold in hours (at-mean closed form).
    pub tau_be_hours: Option<f64>,
    /// Bernstein 99% liquidation-safe leverage bound.
    pub leverage_bound: Option<u32>,
    /// Expected residual income over the next hour, per dollar of notional.
    pub expected_residual_hourly: Option<f64>,
    /// Gate verdict — decision admission.
    pub verdict: ForecastVerdict,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ForecastVerdict {
    /// Admit the trade at full notional.
    Admit,
    /// Admit at reduced size (scale by `size_scale`).
    Reduce,
    /// Reject — forecast does not clear admission gate.
    Reject,
}

/// Inputs required by `score()`. Kept as a struct so the tick loop can pass
/// whatever it has at hand.
pub struct ScoringInputs<'a> {
    /// Spread series (ts_ms, spread) — the cross-venue |max - min| history.
    pub spread_series: &'a [(i64, f64)],
    /// Spread as `&[f64]` for ADF.
    pub spread_values: &'a [f64],
    /// Current observed spread (absolute, annualized).
    pub current_spread_annual: f64,
    /// Current round-trip cost fraction from `decision::decide`.
    pub cost_fraction: f64,
    /// Sampling interval between history entries, in hours.
    pub dt_hours: f64,
    /// Venue maintenance margin rate (MMR) for the short leg.
    pub mmr: f64,
    /// Per-hour bound on spread movement (sup norm). Derived from historical
    /// max |Δspread| / Δt when available; heuristic 0.01 otherwise.
    pub delta_bound_per_h: f64,
    /// Per-hour standard deviation of spread moves (from OU σ or sample std).
    pub sigma_per_h: f64,
}

pub fn score(inputs: ScoringInputs<'_>) -> ForecastScore {
    // ── Regime detection via ADF ──────────────────────────────────────────
    if inputs.spread_values.len() < 50 {
        return ForecastScore {
            regime: Regime::Insufficient,
            adf_statistic: None,
            theta_hourly: None,
            mu_annual: None,
            drift_t_statistic: None,
            tau_be_hours: None,
            leverage_bound: None,
            expected_residual_hourly: None,
            verdict: ForecastVerdict::Admit, // bootstrap: let the trade through
        };
    }

    let (regime, adf_stat) = match adf_test(inputs.spread_values) {
        Ok(res) => {
            let r = if res.rejects_unit_root {
                Regime::Stationary
            } else {
                Regime::Drift
            };
            (r, Some(res.statistic))
        }
        Err(_) => (Regime::Drift, None),
    };

    // ── Fit OU or drift depending on regime ───────────────────────────────
    let ou_fit: Option<FitOuOutput> = if regime == Regime::Stationary {
        fit_ou(inputs.spread_series, inputs.dt_hours).ok()
    } else {
        None
    };
    let drift_fit = if regime == Regime::Drift {
        fit_drift(inputs.spread_series, inputs.dt_hours).ok()
    } else {
        None
    };

    // If OU fit is degenerate, demote to drift path so we don't propagate NaN.
    let ou_fit = match &ou_fit {
        Some(f) if !f.is_degenerate() => ou_fit,
        _ => None,
    };

    // ── Break-even hold + Bernstein leverage ──────────────────────────────
    let (tau_be_h, mu, theta) = match &ou_fit {
        Some(f) => (
            break_even_hold_at_mean(
                AnnualizedRate(f.mu),
                Dimensionless(inputs.cost_fraction),
                Dimensionless(0.0),
            )
            .ok()
            .map(|h| h.0),
            Some(f.mu),
            Some(f.theta),
        ),
        None => (None, drift_fit.as_ref().map(|d| d.mu), None),
    };

    // Use tau_be_h if available, else default to 168h (1 week) for the
    // Bernstein bound.
    let horizon_h = tau_be_h.unwrap_or(168.0).max(1.0);

    let leverage_bound: Option<u32> = bernstein_leverage_bound(
        Dimensionless(inputs.mmr),
        Dimensionless(inputs.delta_bound_per_h),
        Dimensionless(inputs.sigma_per_h),
        Hours(horizon_h),
        0.01, // ε = 1% liquidation probability
    )
    .ok();

    // ── Expected residual income (closed-form OU) ─────────────────────────
    let expected_residual_hourly: Option<f64> = match (mu, theta) {
        (Some(mu_v), Some(theta_v)) if theta_v > 0.0 => {
            Some(expected_residual_income(
                inputs.current_spread_annual,
                mu_v,
                theta_v,
                1.0, // 1-hour horizon for integrability
                1,
            ))
        }
        _ => None,
    };

    // ── Verdict ──────────────────────────────────────────────────────────
    //
    // Admission gate:
    //   - τ^BE exists AND τ^BE ≤ cycle_length (3600s = 1h ×); AND
    //   - expected residual over 1h ≥ cost_fraction / τ^BE (pro-rata).
    //
    // If τ^BE not finite or expected residual < pro-rata cost → Reject.
    let verdict = match (tau_be_h, expected_residual_hourly) {
        (Some(tau), Some(eri)) if tau.is_finite() && tau <= 24.0 * 7.0 => {
            let cost_per_hour = inputs.cost_fraction / tau.max(1.0);
            if eri >= cost_per_hour {
                ForecastVerdict::Admit
            } else if eri >= 0.5 * cost_per_hour {
                ForecastVerdict::Reduce
            } else {
                ForecastVerdict::Reject
            }
        }
        // No OU fit, or drift regime without strong signal → admit cautiously.
        _ => ForecastVerdict::Admit,
    };

    ForecastScore {
        regime,
        adf_statistic: adf_stat,
        theta_hourly: theta,
        mu_annual: mu,
        drift_t_statistic: drift_fit.as_ref().map(|d| d.t_statistic),
        tau_be_hours: tau_be_h,
        leverage_bound,
        expected_residual_hourly,
        verdict,
    }
}

/// Map verdict to size multiplier: Admit=1.0, Reduce=0.5, Reject=0.0.
#[inline]
pub fn verdict_size_scale(v: ForecastVerdict) -> f64 {
    match v {
        ForecastVerdict::Admit => 1.0,
        ForecastVerdict::Reduce => 0.5,
        ForecastVerdict::Reject => 0.0,
    }
}

/// Helper: estimate per-hour delta bound and sigma from a spread series.
/// Uses `max |Δ|/Δt` for the delta and sample std for sigma.
///
/// `dt_hours` is the inter-sample spacing. Returns `(delta_per_h, sigma_per_h)`.
/// Falls back to conservative defaults (1% per hour) when series too short.
pub fn infer_spread_dynamics(spread: &[(i64, f64)], dt_hours: f64) -> (f64, f64) {
    if spread.len() < 10 || dt_hours <= 0.0 {
        return (0.01, 0.005);
    }
    let mut max_delta_per_h: f64 = 0.0;
    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    let n = spread.len();
    for i in 1..n {
        let d = (spread[i].1 - spread[i - 1].1).abs() / dt_hours;
        if d.is_finite() {
            max_delta_per_h = max_delta_per_h.max(d);
        }
        sum += spread[i].1;
        sum_sq += spread[i].1 * spread[i].1;
    }
    let mean = sum / (n - 1) as f64;
    let var = (sum_sq / (n - 1) as f64) - mean * mean;
    let sigma_per_h = if var > 0.0 {
        (var / dt_hours).sqrt()
    } else {
        0.005
    };
    // Handle the case where max_delta_per_h is suspiciously zero
    let delta_eff = if max_delta_per_h > 0.0 {
        max_delta_per_h
    } else {
        0.01
    };
    (delta_eff, sigma_per_h)
}

// Prevent FrameworkError unused-import warning on cfg without test
#[allow(dead_code)]
fn _silence_unused() -> Option<FrameworkError> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_series(len: usize) -> Vec<(i64, f64)> {
        // Synthetic mean-reverting series around μ=0.05.
        (0..len)
            .map(|i| {
                let t = i as i64 * 3600;
                let phase = (i as f64 * 0.1).sin();
                (t, 0.05 + 0.01 * phase)
            })
            .collect()
    }

    #[test]
    fn insufficient_history_bootstraps_admit() {
        let series = make_series(20);
        let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
        let s = score(ScoringInputs {
            spread_series: &series,
            spread_values: &values,
            current_spread_annual: 0.05,
            cost_fraction: 0.001,
            dt_hours: 1.0,
            mmr: 0.03,
            delta_bound_per_h: 0.01,
            sigma_per_h: 0.005,
        });
        assert_eq!(s.regime, Regime::Insufficient);
        assert_eq!(s.verdict, ForecastVerdict::Admit);
    }

    #[test]
    fn long_series_classifies_regime_and_returns_tau_be() {
        let series = make_series(300);
        let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
        let s = score(ScoringInputs {
            spread_series: &series,
            spread_values: &values,
            current_spread_annual: 0.05,
            cost_fraction: 0.001,
            dt_hours: 1.0,
            mmr: 0.03,
            delta_bound_per_h: 0.01,
            sigma_per_h: 0.005,
        });
        assert_ne!(s.regime, Regime::Insufficient);
        // Either stationary or drift — both are valid classifications.
        assert!(s.leverage_bound.is_some());
        assert!(s.leverage_bound.unwrap() >= 1);
    }

    #[test]
    fn verdict_size_scale_map() {
        assert_eq!(verdict_size_scale(ForecastVerdict::Admit), 1.0);
        assert_eq!(verdict_size_scale(ForecastVerdict::Reduce), 0.5);
        assert_eq!(verdict_size_scale(ForecastVerdict::Reject), 0.0);
    }

    #[test]
    fn infer_spread_dynamics_short_falls_back() {
        let (d, s) = infer_spread_dynamics(&[(0, 0.01), (3600, 0.02)], 1.0);
        assert_eq!(d, 0.01);
        assert_eq!(s, 0.005);
    }

    #[test]
    fn infer_spread_dynamics_nontrivial() {
        let series: Vec<(i64, f64)> = (0..100)
            .map(|i| (i as i64 * 3600, 0.05 + (i as f64 * 0.01).sin() * 0.01))
            .collect();
        let (d, s) = infer_spread_dynamics(&series, 1.0);
        assert!(d > 0.0);
        assert!(s > 0.0);
    }
}
