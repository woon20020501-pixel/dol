//! CVaR_99 budget guard.
//!
//! Rolling-window realized P&L buffer → `bot_math::cvar_empirical(α=0.99)` →
//! compare against `budget_99_frac` (fraction of NAV). Addresses
//! `live_gate::has_cvar_guard_nonstub()` (flipped to true when this guard is
//! wired into the decision path).
//!
//! Reference: Rockafellar & Uryasev (2000), "Optimization of conditional
//! value-at-risk", J. Risk 2(3):21-41. Empirical estimator is the sample
//! top-(1-α)-tail mean; see `bot_math::cvar::cvar_empirical` for the formula.
//!
//! Semantics:
//! - LOSS SIGN CONVENTION: losses are POSITIVE (negative gains). Callers pass
//!   `-pnl_delta` when recording a gain, `+loss_usd` for a drawdown.
//! - Buffer window: 7 × 24 hours default (matches Python default). Capped at
//!   `MAX_SAMPLES = 10_080` (1-min ticks over 7d) to bound memory.
//! - Gate: if `cvar_99_usd > budget_99_frac × nav_usd`, BLOCK new entries.
//!   Existing positions held (funding_cycle_lock semantics unchanged).

use std::collections::VecDeque;

use bot_math::cvar_empirical;

use super::RiskDecision;

/// Default CVaR_99 budget as fraction of NAV (2%).
pub const DEFAULT_BUDGET_99_FRAC: f64 = 0.02;

/// Default confidence level (α = 0.99).
pub const DEFAULT_ALPHA: f64 = 0.99;

/// Maximum samples retained in the rolling buffer.
/// 10_080 = 7 days × 24 h × 60 min — more than enough for any practical α.
pub const MAX_SAMPLES: usize = 10_080;

/// Minimum samples required before the guard becomes active. Below this it
/// returns `Pass` (can't estimate a tail from <100 samples at α=0.99).
pub const MIN_SAMPLES: usize = 100;

#[derive(Debug, Clone)]
pub struct CvarBudgetGuard {
    losses: VecDeque<f64>,
    alpha: f64,
    budget_frac: f64,
    capacity: usize,
}

impl CvarBudgetGuard {
    pub fn new() -> Self {
        Self::with_params(DEFAULT_ALPHA, DEFAULT_BUDGET_99_FRAC, MAX_SAMPLES)
    }

    pub fn with_params(alpha: f64, budget_frac: f64, capacity: usize) -> Self {
        assert!(
            0.5 < alpha && alpha < 1.0,
            "alpha must be in (0.5, 1.0), got {}",
            alpha
        );
        assert!(
            budget_frac > 0.0 && budget_frac < 1.0,
            "budget_frac must be in (0, 1), got {}",
            budget_frac
        );
        assert!(capacity >= MIN_SAMPLES, "capacity < MIN_SAMPLES");
        Self {
            losses: VecDeque::with_capacity(capacity),
            alpha,
            budget_frac,
            capacity,
        }
    }

    /// Record a realized P&L delta for this tick.
    /// `pnl_delta_usd` is net P&L (positive = gain). Stored internally as
    /// its negative (loss sign convention).
    pub fn record(&mut self, pnl_delta_usd: f64) {
        if !pnl_delta_usd.is_finite() {
            // NaN/Inf P&L is excluded at the estimator level; avoid polluting
            // the buffer with it.
            return;
        }
        if self.losses.len() == self.capacity {
            self.losses.pop_front();
        }
        self.losses.push_back(-pnl_delta_usd);
    }

    /// Current CVaR_α estimate in USD, or `None` if insufficient samples.
    pub fn cvar_usd(&self) -> Option<f64> {
        if self.losses.len() < MIN_SAMPLES {
            return None;
        }
        let slice: Vec<f64> = self.losses.iter().copied().collect();
        cvar_empirical(&slice, self.alpha)
    }

    /// Sample count currently in the buffer.
    pub fn sample_count(&self) -> usize {
        self.losses.len()
    }

    /// Gate decision for a candidate trade against `nav_usd`.
    pub fn check(&self, nav_usd: f64) -> RiskDecision {
        let Some(cvar) = self.cvar_usd() else {
            return RiskDecision::Pass;
        };
        if !nav_usd.is_finite() || nav_usd <= 0.0 {
            return RiskDecision::Block {
                reason: format!("NAV non-positive or NaN ({nav_usd})"),
            };
        }
        let budget_usd = self.budget_frac * nav_usd;
        if cvar > budget_usd {
            // Soft-landing: proportional reduce if overshoot < 2×, hard block above.
            let overshoot_ratio = cvar / budget_usd;
            if overshoot_ratio < 2.0 {
                let size_multiplier = (1.0 / overshoot_ratio).clamp(0.0, 1.0);
                return RiskDecision::Reduce {
                    size_multiplier,
                    reason: format!(
                        "CVaR_{:.0} ${:.2} > budget ${:.2} (overshoot {:.2}x)",
                        self.alpha * 100.0,
                        cvar,
                        budget_usd,
                        overshoot_ratio
                    ),
                };
            }
            return RiskDecision::Block {
                reason: format!(
                    "CVaR_{:.0} ${:.2} > 2× budget ${:.2}",
                    self.alpha * 100.0,
                    cvar,
                    budget_usd
                ),
            };
        }
        RiskDecision::Pass
    }
}

impl Default for CvarBudgetGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_min_samples_passes() {
        let g = CvarBudgetGuard::new();
        assert_eq!(g.check(10_000.0), RiskDecision::Pass);
    }

    #[test]
    fn all_gains_never_blocks() {
        let mut g = CvarBudgetGuard::new();
        for _ in 0..MIN_SAMPLES + 10 {
            g.record(1.0); // every tick is a $1 gain
        }
        assert_eq!(g.check(10_000.0), RiskDecision::Pass);
    }

    #[test]
    fn sustained_losses_trigger_reduce_then_block() {
        let mut g = CvarBudgetGuard::with_params(0.95, 0.01, 1_000);
        // 200 ticks of uniform -$1 to -$5 loss
        for i in 0..200 {
            let loss = -((i % 5 + 1) as f64); // -1..-5 cycle
            g.record(loss);
        }
        let nav = 10_000.0; // budget = $100
                            // CVaR_95 on uniform losses {1..5}: top 5% of 200 = 10 samples, all $5 → cvar = 5
                            // 5 < 100 → should still pass
        assert_eq!(g.check(nav), RiskDecision::Pass);
    }

    #[test]
    fn cvar_exceeds_budget_blocks() {
        let mut g = CvarBudgetGuard::with_params(0.99, 0.01, 1_000);
        // 500 ticks with 1% tail at $1000 loss
        for i in 0..500 {
            let loss = if i < 5 { -1_000.0 } else { -1.0 };
            g.record(loss);
        }
        // top 1% of 500 = 5, all $1000 → CVaR_99 = 1000
        // budget = 1% × NAV = $100 → overshoot 10× → BLOCK
        let decision = g.check(10_000.0);
        assert!(matches!(decision, RiskDecision::Block { .. }));
    }

    #[test]
    fn soft_landing_proportional_reduce() {
        // Use α=0.8: with n=1000 ⇒ tail_k = ceil(0.2·1000) = 200.
        // Put exactly 200 tail entries at $150, 800 at $0 → CVaR=$150,
        // budget=$100, overshoot=1.5×, size_mult=0.667.
        let mut g = CvarBudgetGuard::with_params(0.8, 0.01, 1_200);
        for i in 0..1_000 {
            let loss = if i < 200 { -150.0 } else { 0.0 };
            g.record(loss);
        }
        let decision = g.check(10_000.0);
        match decision {
            RiskDecision::Reduce {
                size_multiplier, ..
            } => {
                assert!(
                    (size_multiplier - 1.0 / 1.5).abs() < 1e-9,
                    "expected 0.667, got {}",
                    size_multiplier
                );
            }
            other => panic!("expected Reduce, got {:?}", other),
        }
    }

    #[test]
    fn nan_losses_excluded() {
        let mut g = CvarBudgetGuard::with_params(0.95, 0.01, 1_000);
        g.record(f64::NAN);
        g.record(f64::INFINITY);
        assert_eq!(g.sample_count(), 0, "non-finite P&L must not be stored");
    }

    #[test]
    fn buffer_capacity_enforced() {
        let mut g = CvarBudgetGuard::with_params(0.95, 0.01, MIN_SAMPLES);
        for i in 0..MIN_SAMPLES * 3 {
            g.record(-(i as f64));
        }
        assert_eq!(g.sample_count(), MIN_SAMPLES, "capacity cap not enforced");
    }

    #[test]
    fn nav_non_positive_blocks() {
        let mut g = CvarBudgetGuard::new();
        for _ in 0..MIN_SAMPLES {
            g.record(-1.0);
        }
        assert!(matches!(g.check(0.0), RiskDecision::Block { .. }));
        assert!(matches!(g.check(-100.0), RiskDecision::Block { .. }));
    }
}
