//! Drawdown stop — wires `bot_strategy_v3::stochastic::cvar_drawdown_stop`
//! into the runtime.
//!
//! Reference: Rockafellar & Uryasev (2000) CVaR + tail-risk drawdown
//! literature. The Python `stochastic.cvar_drawdown_stop` returns an
//! **absolute** drawdown threshold (fraction of notional) derived from the
//! empirical 99th-percentile tail of historical |basis divergence|.
//!
//! Runtime behavior:
//! 1. Peak-NAV tracker records the running maximum `nav_usd`.
//! 2. Current drawdown = (peak - current) / peak.
//! 3. If `drawdown > α · cvar_stop`, escalate.
//!
//! Policy:
//! - `dd < 0.5 · cvar_stop`                     → Pass
//! - `0.5 · cvar_stop ≤ dd < cvar_stop`         → Reduce 0.5
//! - `dd ≥ cvar_stop`                           → Flatten
//!
//! The basis history feeding `cvar_drawdown_stop` is caller-managed — the
//! guard is stateless beyond the NAV peak tracker. Tests use a deterministic
//! fallback (the Python bootstrap: 0.005 when history too short).

use bot_strategy_v3::stochastic::cvar_drawdown_stop;

use super::RiskDecision;

/// Default confidence α passed through to `cvar_drawdown_stop`.
pub const DEFAULT_ALPHA: f64 = 0.01;

/// Default safety multiplier (see Python default `safety_multiplier=2.0`).
pub const DEFAULT_SAFETY_MULTIPLIER: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct DrawdownStop {
    peak_nav: f64,
    alpha: f64,
    safety_multiplier: f64,
}

impl DrawdownStop {
    pub fn new(initial_nav: f64) -> Self {
        Self::with_params(initial_nav, DEFAULT_ALPHA, DEFAULT_SAFETY_MULTIPLIER)
    }

    pub fn with_params(initial_nav: f64, alpha: f64, multiplier: f64) -> Self {
        assert!(initial_nav > 0.0, "initial NAV must be positive");
        assert!(0.0 < alpha && alpha < 1.0);
        assert!(multiplier > 0.0);
        Self {
            peak_nav: initial_nav,
            alpha,
            safety_multiplier: multiplier,
        }
    }

    /// Update the peak-NAV tracker.
    pub fn record_nav(&mut self, nav_usd: f64) {
        if nav_usd.is_finite() && nav_usd > self.peak_nav {
            self.peak_nav = nav_usd;
        }
    }

    pub fn peak_nav(&self) -> f64 {
        self.peak_nav
    }

    /// Drawdown from peak as a non-negative fraction.
    pub fn drawdown_fraction(&self, nav_usd: f64) -> f64 {
        if self.peak_nav <= 0.0 || !nav_usd.is_finite() {
            return 0.0;
        }
        ((self.peak_nav - nav_usd) / self.peak_nav).max(0.0)
    }

    /// Evaluate drawdown against the CVaR-derived stop.
    ///
    /// `basis_history` is the (ts_ms, basis_divergence) series used to
    /// estimate the tail stop via `cvar_drawdown_stop`. Pass an empty slice
    /// to get the Python bootstrap fallback (0.005 = 50 bps).
    pub fn check(&self, nav_usd: f64, basis_history: &[(i64, f64)]) -> RiskDecision {
        let stop = cvar_drawdown_stop(basis_history, self.alpha, self.safety_multiplier);
        if !stop.is_finite() || stop <= 0.0 {
            return RiskDecision::Pass;
        }
        let dd = self.drawdown_fraction(nav_usd);
        if dd < 0.5 * stop {
            return RiskDecision::Pass;
        }
        if dd < stop {
            return RiskDecision::Reduce {
                size_multiplier: 0.5,
                reason: format!(
                    "drawdown {:.3}% ∈ [0.5×stop, stop) (stop={:.3}%)",
                    dd * 100.0,
                    stop * 100.0
                ),
            };
        }
        RiskDecision::Flatten {
            reason: format!(
                "drawdown {:.3}% ≥ cvar stop {:.3}% (peak ${:.2}, now ${:.2})",
                dd * 100.0,
                stop * 100.0,
                self.peak_nav,
                nav_usd
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_at_peak_passes() {
        let g = DrawdownStop::new(10_000.0);
        assert_eq!(g.check(10_000.0, &[]), RiskDecision::Pass);
    }

    #[test]
    fn peak_tracks_high_water_mark() {
        let mut g = DrawdownStop::new(10_000.0);
        g.record_nav(11_000.0);
        g.record_nav(10_500.0); // drop
        assert_eq!(g.peak_nav, 11_000.0);
        assert!((g.drawdown_fraction(10_500.0) - 500.0 / 11_000.0).abs() < 1e-12);
    }

    #[test]
    fn moderate_drawdown_triggers_reduce() {
        // Bootstrap stop = 0.005 (0.5%).
        // Need 0.25% < dd < 0.5% → reduce.
        let g = DrawdownStop::new(10_000.0);
        let nav = 10_000.0 * (1.0 - 0.004); // 0.4% dd
        match g.check(nav, &[]) {
            RiskDecision::Reduce { .. } => {}
            other => panic!("expected Reduce, got {:?}", other),
        }
    }

    #[test]
    fn large_drawdown_flattens() {
        let g = DrawdownStop::new(10_000.0);
        // Bootstrap stop = 0.5%. Any dd above that flattens.
        let nav = 10_000.0 * (1.0 - 0.02); // 2% dd
        assert!(matches!(g.check(nav, &[]), RiskDecision::Flatten { .. }));
    }

    #[test]
    fn peak_does_not_drop() {
        let mut g = DrawdownStop::new(10_000.0);
        g.record_nav(9_000.0);
        assert_eq!(g.peak_nav, 10_000.0);
    }

    #[test]
    fn nan_nav_is_treated_as_no_drawdown() {
        let g = DrawdownStop::new(10_000.0);
        assert_eq!(g.drawdown_fraction(f64::NAN), 0.0);
    }
}
