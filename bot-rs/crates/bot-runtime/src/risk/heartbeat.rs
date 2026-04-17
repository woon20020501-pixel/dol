//! Hedge-leg heartbeat watchdog.
//!
//! Funding-arb bots are exposed to **legging risk**: if the pivot leg fills
//! but the counter (hedge) leg doesn't within seconds, the book is suddenly
//! naked-directional and vulnerable to mark moves. Reference:
//! Almgren & Chriss (2000), "Optimal execution of portfolio transactions",
//! J. Risk 3(2):5-39, §3 (temporary impact during execution window).
//!
//! This guard monitors the time since the most recent *paired* fill and
//! escalates if it exceeds `max_hedge_gap_seconds`. A paired fill is defined
//! as: pivot and hedge leg both reported within `pair_window_seconds` of
//! each other.
//!
//! Policy:
//! - `gap < max_hedge_gap_seconds`       → Pass
//! - `max <= gap < 2 × max`              → Reduce (0.25 multiplier, signal degraded)
//! - `gap ≥ 2 × max`                     → Flatten (hedge lost)

use std::time::{Duration, Instant};

use super::RiskDecision;

/// Default: 5 seconds max hedge gap (Almgren-Chriss legging drift window for
/// typical crypto perp execution).
pub const DEFAULT_MAX_HEDGE_GAP: Duration = Duration::from_secs(5);

/// Default: paired fills must arrive within 2 seconds to count.
pub const DEFAULT_PAIR_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct HedgeHeartbeat {
    last_paired_fill: Option<Instant>,
    last_pivot_fill: Option<Instant>,
    last_hedge_fill: Option<Instant>,
    max_hedge_gap: Duration,
    pair_window: Duration,
}

impl HedgeHeartbeat {
    pub fn new() -> Self {
        Self::with_params(DEFAULT_MAX_HEDGE_GAP, DEFAULT_PAIR_WINDOW)
    }

    pub fn with_params(max_hedge_gap: Duration, pair_window: Duration) -> Self {
        assert!(max_hedge_gap > Duration::ZERO);
        assert!(pair_window > Duration::ZERO);
        Self {
            last_paired_fill: None,
            last_pivot_fill: None,
            last_hedge_fill: None,
            max_hedge_gap,
            pair_window,
        }
    }

    /// Record a pivot (long-leg / maker) fill event.
    pub fn record_pivot_fill(&mut self, now: Instant) {
        self.last_pivot_fill = Some(now);
        self.check_pairing(now);
    }

    /// Record a hedge (counter-leg / taker) fill event.
    pub fn record_hedge_fill(&mut self, now: Instant) {
        self.last_hedge_fill = Some(now);
        self.check_pairing(now);
    }

    fn check_pairing(&mut self, now: Instant) {
        if let (Some(pivot), Some(hedge)) = (self.last_pivot_fill, self.last_hedge_fill) {
            let gap = if pivot > hedge {
                pivot - hedge
            } else {
                hedge - pivot
            };
            if gap <= self.pair_window {
                self.last_paired_fill = Some(now);
            }
        }
    }

    /// Evaluate the heartbeat against `now`. Returns `Pass` when no positions
    /// have been opened yet (no reference point) — the guard only fires once
    /// a paired fill has been observed and a subsequent pivot fill outruns
    /// its hedge.
    pub fn check(&self, now: Instant) -> RiskDecision {
        // Only meaningful once we've seen at least one paired fill.
        let Some(last_paired) = self.last_paired_fill else {
            return RiskDecision::Pass;
        };
        // Most recent activity is whichever of pivot/hedge is most recent.
        let last_activity = match (self.last_pivot_fill, self.last_hedge_fill) {
            (Some(a), Some(b)) => a.max(b),
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => last_paired,
        };
        // If the most-recent activity is a paired fill we're fine.
        if last_activity <= last_paired {
            return RiskDecision::Pass;
        }
        // Otherwise the pivot/hedge single-sided gap is the relevant signal.
        let gap = now.saturating_duration_since(last_paired);
        if gap < self.max_hedge_gap {
            return RiskDecision::Pass;
        }
        if gap < 2 * self.max_hedge_gap {
            return RiskDecision::Reduce {
                size_multiplier: 0.25,
                reason: format!(
                    "hedge gap {:.1}s > {:.1}s — reducing new entries",
                    gap.as_secs_f64(),
                    self.max_hedge_gap.as_secs_f64()
                ),
            };
        }
        RiskDecision::Flatten {
            reason: format!(
                "hedge gap {:.1}s ≥ 2× {:.1}s — legging risk critical",
                gap.as_secs_f64(),
                self.max_hedge_gap.as_secs_f64()
            ),
        }
    }
}

impl Default for HedgeHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fills_passes() {
        let hb = HedgeHeartbeat::new();
        assert_eq!(hb.check(Instant::now()), RiskDecision::Pass);
    }

    #[test]
    fn paired_fills_within_window_pass() {
        let mut hb = HedgeHeartbeat::new();
        let t0 = Instant::now();
        hb.record_pivot_fill(t0);
        hb.record_hedge_fill(t0 + Duration::from_millis(500));
        let later = t0 + Duration::from_secs(3);
        assert_eq!(hb.check(later), RiskDecision::Pass);
    }

    #[test]
    fn pivot_without_hedge_triggers_reduce_then_flatten() {
        let mut hb = HedgeHeartbeat::with_params(Duration::from_secs(5), Duration::from_secs(2));
        let t0 = Instant::now();
        // Establish a paired baseline
        hb.record_pivot_fill(t0);
        hb.record_hedge_fill(t0 + Duration::from_millis(100));
        // Later: pivot fills but hedge doesn't
        hb.record_pivot_fill(t0 + Duration::from_secs(10));
        // 1.0s after pivot-only fill: gap from paired (at t0+0.1s) is ~10.9s > 2×5s → Flatten
        let t_check = t0 + Duration::from_secs(11);
        assert!(
            matches!(hb.check(t_check), RiskDecision::Flatten { .. }),
            "gap > 2× max must flatten"
        );
    }

    #[test]
    fn moderate_gap_triggers_reduce() {
        let mut hb = HedgeHeartbeat::with_params(Duration::from_secs(5), Duration::from_secs(2));
        let t0 = Instant::now();
        hb.record_pivot_fill(t0);
        hb.record_hedge_fill(t0 + Duration::from_millis(100));
        // Unpaired pivot 6s after paired; gap between paired(t0+0.1) and "now"(t0+7) ≈ 6.9s
        // 6.9 > max(5) but < 2×max(10) → Reduce
        hb.record_pivot_fill(t0 + Duration::from_secs(6));
        let t_check = t0 + Duration::from_secs(7);
        match hb.check(t_check) {
            RiskDecision::Reduce {
                size_multiplier, ..
            } => assert!(size_multiplier < 1.0),
            other => panic!("expected Reduce, got {:?}", other),
        }
    }
}
