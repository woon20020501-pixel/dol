//! Per-symbol adapter health telemetry.
//!
//! Tracks the "drops-and-continues" failure behavior of the tick engine so
//! the dashboard can render a yellow/red warning glyph on symbols
//! whose Pacifica `/book` endpoint has been flaky. Does NOT implement a
//! retry-and-degrade policy; `run_one_tick` already drops the failing
//! venue and proceeds with the others, so demo safety is preserved.
//! This module only adds the counter rollup for telemetry.
//!
//! If the policy ever needs to escalate to (b) — the full retry-and-
//! degrade path — extend `SymbolHealth` with `cooldown_remaining`
//! / `is_degraded_streak` fields and add a `tick_with_policy` method.
//! The current struct shape is forward-compatible with that upgrade.

use std::collections::BTreeMap;

use serde::Serialize;

use bot_types::Venue;

/// Per-symbol health counter. One instance per symbol tracked in the
/// `AdapterHealthRegistry`. Serializable into the signal JSON.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SymbolHealth {
    /// Total number of venue-fetch failures observed for this symbol
    /// since the bot started. Sum over all venues.
    pub total_failures: u32,
    /// Consecutive ticks ending in at least one venue-fetch failure.
    /// Resets to 0 on a tick where every venue fetch succeeded.
    pub consecutive_failures: u32,
    /// True iff `consecutive_failures >= DEGRADATION_THRESHOLD`. The
    /// dashboard renders a red dot when this is true. Note: demo mode does
    /// NOT exclude the symbol from the decision loop when degraded — that
    /// is the full retry-and-degrade policy which has not been implemented.
    /// The flag is advisory telemetry only in v0.
    pub is_degraded: bool,
    /// Last venue that failed to fetch for this symbol, as a readable
    /// string (`"Pacifica"`, `"Hyperliquid"`, ...). `None` if no failure
    /// has occurred yet.
    pub last_failure_venue: Option<String>,
}

/// Consecutive-failure threshold that flips `is_degraded = true`.
pub const DEGRADATION_THRESHOLD: u32 = 3;

/// Registry of per-symbol adapter health, keyed by symbol string.
///
/// The caller (tick loop) updates the registry each tick via
/// [`AdapterHealthRegistry::record_tick`]. The signal emitter reads via [`AdapterHealthRegistry::health_for`].
#[derive(Debug, Default)]
pub struct AdapterHealthRegistry {
    by_symbol: BTreeMap<String, SymbolHealth>,
}

impl AdapterHealthRegistry {
    pub fn new() -> Self {
        Self {
            by_symbol: BTreeMap::new(),
        }
    }

    /// Ensure a health entry exists for `symbol` and return a mutable
    /// reference. Creates a default entry if missing.
    fn entry(&mut self, symbol: &str) -> &mut SymbolHealth {
        self.by_symbol.entry(symbol.to_string()).or_default()
    }

    /// Record the outcome of a single tick's fetch attempts for one symbol.
    ///
    /// `failures` is the list of `(venue, reason)` pairs whose adapter
    /// fetch errored this tick. An empty list means all venues succeeded.
    pub fn record_tick(&mut self, symbol: &str, failures: &[(Venue, &str)]) {
        let h = self.entry(symbol);
        if failures.is_empty() {
            // All venues succeeded — reset the consecutive counter.
            h.consecutive_failures = 0;
            h.is_degraded = false;
            // Leave total_failures and last_failure_venue as historical record.
        } else {
            h.total_failures = h.total_failures.saturating_add(failures.len() as u32);
            h.consecutive_failures = h.consecutive_failures.saturating_add(1);
            if h.consecutive_failures >= DEGRADATION_THRESHOLD {
                h.is_degraded = true;
            }
            // Record the first failing venue this tick for diagnostics.
            h.last_failure_venue = Some(format!("{:?}", failures[0].0));
        }
    }

    /// Read-only snapshot of the current health for `symbol`. Returns a
    /// clone so callers can hand it to `emit_signal` without borrow trouble.
    /// Returns a default all-zero struct if the symbol has no history.
    pub fn health_for(&self, symbol: &str) -> SymbolHealth {
        self.by_symbol.get(symbol).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_health_is_all_zero() {
        let reg = AdapterHealthRegistry::new();
        let h = reg.health_for("BTC");
        assert_eq!(h.total_failures, 0);
        assert_eq!(h.consecutive_failures, 0);
        assert!(!h.is_degraded);
        assert!(h.last_failure_venue.is_none());
    }

    #[test]
    fn success_tick_keeps_zero() {
        let mut reg = AdapterHealthRegistry::new();
        reg.record_tick("BTC", &[]);
        let h = reg.health_for("BTC");
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.total_failures, 0);
        assert!(!h.is_degraded);
    }

    #[test]
    fn single_failure_increments_counters() {
        let mut reg = AdapterHealthRegistry::new();
        reg.record_tick("PAXG", &[(Venue::Pacifica, "parse error")]);
        let h = reg.health_for("PAXG");
        assert_eq!(h.total_failures, 1);
        assert_eq!(h.consecutive_failures, 1);
        assert!(!h.is_degraded); // still below threshold
        assert_eq!(h.last_failure_venue.as_deref(), Some("Pacifica"));
    }

    #[test]
    fn three_consecutive_failures_trip_degraded() {
        let mut reg = AdapterHealthRegistry::new();
        for _ in 0..3 {
            reg.record_tick("PAXG", &[(Venue::Pacifica, "parse error")]);
        }
        let h = reg.health_for("PAXG");
        assert_eq!(h.consecutive_failures, 3);
        assert!(h.is_degraded);
    }

    #[test]
    fn success_after_failures_resets_consecutive_and_clears_degraded() {
        let mut reg = AdapterHealthRegistry::new();
        for _ in 0..3 {
            reg.record_tick("PAXG", &[(Venue::Pacifica, "parse error")]);
        }
        assert!(reg.health_for("PAXG").is_degraded);
        reg.record_tick("PAXG", &[]);
        let h = reg.health_for("PAXG");
        assert_eq!(h.consecutive_failures, 0);
        assert!(!h.is_degraded);
        // Total failures is historical and should persist.
        assert_eq!(h.total_failures, 3);
    }

    #[test]
    fn multiple_failures_in_one_tick_count_individually_for_total() {
        let mut reg = AdapterHealthRegistry::new();
        reg.record_tick(
            "XAU",
            &[(Venue::Pacifica, "err1"), (Venue::Hyperliquid, "err2")],
        );
        let h = reg.health_for("XAU");
        assert_eq!(h.total_failures, 2);
        // But only ONE consecutive tick ended in failure.
        assert_eq!(h.consecutive_failures, 1);
    }
}
