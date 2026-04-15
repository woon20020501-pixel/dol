//! Runtime adapter around `bot_strategy_v3::funding_cycle_lock`.
//!
//! Maintains per-symbol `CycleState` and threads every `PairDecision` through
//! `funding_cycle_lock::enforce()` so that I-LOCK (iron law §1) is honored
//! on the demo decision path. When live order submission is wired in Week 2+,
//! the enforcement call is already in the call graph — there is no escape
//! hatch to plug.
//!
//! # Direction encoding
//!
//! `funding_cycle_lock` uses a scalar `h_c ∈ {-1, 0, +1}`. We map a
//! `PairDecision`'s `(long_venue, short_venue)` tuple onto `h_c` via a
//! canonical convention: `+1` if `long_venue` has a LOWER enum ordinal
//! than `short_venue`, `-1` otherwise. This guarantees that a direction
//! flip (long A, short B) → (long B, short A) also flips `h_c` and is
//! caught by `would_violate_lock`.
//!
//! **Known limitation:** two distinct pair-pair combinations with the same
//! canonical ordering (e.g. long-Pacifica/short-Hyperliquid vs
//! long-Pacifica/short-Lighter) share the same `h_c`. A mid-cycle rebalance
//! between those is NOT detected by the scalar lock alone. We compensate by
//! also storing the full locked `PairDecision` and comparing
//! `(long, short, symbol)` identity on held ticks. See `EnforceOutcome`
//! below.

use std::collections::BTreeMap;

use tracing::warn;

use bot_strategy_v3::funding_cycle_lock::{
    enforce as fcl_enforce, CycleState, EnforceResult, DEFAULT_CYCLE_SECONDS,
};
use bot_types::Venue;

use crate::decision::PairDecision;

/// Outcome of enforcing the lock on a proposed decision.
#[derive(Debug, Clone)]
pub struct EnforceOutcome {
    /// The decision the bot should act on. May differ from the proposed
    /// one if a lock is held.
    pub effective: Option<PairDecision>,
    /// Raw result from `funding_cycle_lock::enforce`.
    pub raw: EnforceResult,
    /// Whether a direction flip or pair rebalance was blocked by the lock.
    pub pair_flip_blocked: bool,
    /// Whether this tick opened a fresh cycle for the symbol.
    pub opened_new_cycle: bool,
}

/// Per-symbol state tracked by the registry: the `CycleState` and the full
/// locked `PairDecision` (used to detect pair rebalances that share the
/// canonical `h_c`).
#[derive(Debug, Clone)]
struct SymbolLock {
    state: CycleState,
    locked_decision: PairDecision,
}

/// Per-symbol cycle lock registry.
#[derive(Debug, Default)]
pub struct CycleLockRegistry {
    by_symbol: BTreeMap<String, SymbolLock>,
    /// Cadence in seconds (default 3600 = Pacifica hourly).
    cycle_seconds: i64,
}

impl CycleLockRegistry {
    pub fn new() -> Self {
        Self {
            by_symbol: BTreeMap::new(),
            cycle_seconds: DEFAULT_CYCLE_SECONDS,
        }
    }

    pub fn with_cycle_seconds(cycle_seconds: i64) -> Self {
        assert!(cycle_seconds > 0, "cycle_seconds must be positive");
        Self {
            by_symbol: BTreeMap::new(),
            cycle_seconds,
        }
    }

    /// Current cycle state for `symbol`, if any. Primarily for diagnostics /
    /// signal-JSON population. Read-only.
    pub fn state_for(&self, symbol: &str) -> Option<CycleState> {
        self.by_symbol.get(symbol).map(|e| e.state)
    }

    /// Currently-locked decision for `symbol`, if any.
    pub fn locked_decision_for(&self, symbol: &str) -> Option<&PairDecision> {
        self.by_symbol.get(symbol).map(|e| &e.locked_decision)
    }

    /// Enforce the cycle lock on `proposed`.
    ///
    /// - If no proposed decision AND a lock is held → returns the locked
    ///   decision (hold through the gap).
    /// - If a proposed decision matches the locked pair identity → returns
    ///   the locked decision with locked notional (demonstrates Aurora-Ω
    ///   §3.1 frozen target_notional semantics).
    /// - If a proposed decision's pair differs from the locked pair (flip
    ///   or rebalance) → lock wins, blocked flag set.
    /// - If no active lock → open new cycle with the proposed (h, N) and
    ///   return the proposed decision.
    ///
    /// `now_s` is Unix seconds (f64).
    pub fn enforce_decision(
        &mut self,
        symbol: &str,
        now_s: f64,
        proposed: Option<&PairDecision>,
        emergency_override: bool,
    ) -> EnforceOutcome {
        // Materialize current state as the Python-shaped Option<CycleState>
        // the underlying `enforce` expects.
        let mut state = self.by_symbol.get(symbol).map(|e| e.state);

        let (proposed_h, proposed_n) = match proposed {
            Some(d) => (pair_to_h(d.long_venue, d.short_venue), d.notional_usd),
            // No proposal → represent as "flat at 0 notional" to drive the
            // enforce() control flow. If a cycle was open, the lock still
            // wins and `effective` becomes the locked decision.
            None => (0i8, 0.0),
        };

        let raw = fcl_enforce(
            &mut state,
            now_s,
            proposed_h,
            proposed_n,
            emergency_override,
            self.cycle_seconds,
        );

        // Side-effect: if enforce opened a new cycle, record the full
        // proposed decision alongside it. Otherwise carry the existing
        // locked decision forward (or clear if enforce nuked the state,
        // which it doesn't currently but we model it for safety).
        let effective: Option<PairDecision>;
        let mut pair_flip_blocked = false;

        if raw.opened_new_cycle {
            // Fresh cycle: store the proposed decision (if any).
            if let Some(d) = proposed {
                let new_state = state.expect("opened_new_cycle ⇒ state is Some");
                self.by_symbol.insert(
                    symbol.to_string(),
                    SymbolLock {
                        state: new_state,
                        locked_decision: d.clone(),
                    },
                );
                effective = Some(d.clone());
            } else {
                // Edge case: enforce() opened a cycle with h=0/N=0. This
                // means the caller proposed flat. Clear any prior lock.
                self.by_symbol.remove(symbol);
                effective = None;
            }
        } else if raw.emergency_override_used {
            // Override bypassed the lock. Caller is responsible for logging.
            // Do NOT update registry state — operator must follow up via
            // a separate unlock path. For the demo we just reflect the
            // proposed decision as effective.
            effective = proposed.cloned();
        } else {
            // Held: the lock is active and the caller's proposal (if any)
            // was either identical (no-op) or blocked (lock wins).
            match (proposed, self.by_symbol.get(symbol)) {
                (Some(p), Some(locked)) => {
                    let identity_match = locked.locked_decision.long_venue == p.long_venue
                        && locked.locked_decision.short_venue == p.short_venue
                        && locked.locked_decision.symbol == p.symbol;
                    if !identity_match {
                        pair_flip_blocked = true;
                        warn!(
                            symbol = %symbol,
                            locked_long = ?locked.locked_decision.long_venue,
                            locked_short = ?locked.locked_decision.short_venue,
                            proposed_long = ?p.long_venue,
                            proposed_short = ?p.short_venue,
                            "I-LOCK: pair flip/rebalance blocked mid-cycle"
                        );
                    }
                    // Either way, effective is the locked decision.
                    // If N_c differs from proposed N, the raw.proposed_was_blocked
                    // flag will already be true and reflected in the final record.
                    effective = Some(locked.locked_decision.clone());
                }
                (None, Some(locked)) => {
                    // HeldThroughGap in NavTracker terms — held on silent tick.
                    effective = Some(locked.locked_decision.clone());
                }
                (_, None) => {
                    // No lock and no open — nothing to do.
                    effective = proposed.cloned();
                }
            }
        }

        EnforceOutcome {
            effective,
            raw,
            pair_flip_blocked,
            opened_new_cycle: raw.opened_new_cycle,
        }
    }
}

/// Canonical venue-ordering → direction scalar.
///
/// `+1` if `long` has a lower enum ordinal than `short`, `-1` otherwise.
/// `0` for the degenerate case where both are the same (should never occur
/// for a valid pair).
pub fn pair_to_h(long: Venue, short: Venue) -> i8 {
    let lo = long as u8;
    let so = short as u8;
    match lo.cmp(&so) {
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Greater => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::PairDecision;

    fn make_decision(long: Venue, short: Venue, notional: f64) -> PairDecision {
        PairDecision {
            long_venue: long,
            short_venue: short,
            symbol: "BTC".to_string(),
            spread_annual: 0.18,
            cost_fraction: 0.0015,
            net_annual: 0.18 - 0.0015,
            notional_usd: notional,
            reason: "test".to_string(),
            would_have_executed: true,
        }
    }

    #[test]
    fn pair_to_h_canonical() {
        // Enum order: Pacifica=0, Backpack=1, Hyperliquid=2, Lighter=3
        assert_eq!(pair_to_h(Venue::Pacifica, Venue::Backpack), 1);
        assert_eq!(pair_to_h(Venue::Backpack, Venue::Pacifica), -1);
        assert_eq!(pair_to_h(Venue::Pacifica, Venue::Lighter), 1);
        assert_eq!(pair_to_h(Venue::Lighter, Venue::Pacifica), -1);
    }

    #[test]
    fn first_tick_opens_cycle() {
        let mut reg = CycleLockRegistry::new();
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        let out = reg.enforce_decision("BTC", 1_700_000_000.0, Some(&d), false);
        assert!(out.opened_new_cycle);
        assert!(!out.pair_flip_blocked);
        assert!(out.effective.is_some());
    }

    #[test]
    fn second_tick_same_pair_holds() {
        let mut reg = CycleLockRegistry::new();
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&d), false);
        let out = reg.enforce_decision("BTC", 1_700_000_500.0, Some(&d), false);
        assert!(!out.opened_new_cycle);
        assert!(!out.pair_flip_blocked);
        let eff = out.effective.expect("should hold locked decision");
        assert_eq!(eff.long_venue, Venue::Pacifica);
        assert_eq!(eff.short_venue, Venue::Backpack);
    }

    #[test]
    fn mid_cycle_flip_is_blocked() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        // Same cycle, different direction
        let flip = make_decision(Venue::Backpack, Venue::Pacifica, 100.0);
        let out = reg.enforce_decision("BTC", 1_700_000_500.0, Some(&flip), false);
        assert!(out.pair_flip_blocked);
        assert!(out.raw.proposed_was_blocked);
        let eff = out.effective.expect("should return locked decision");
        assert_eq!(eff.long_venue, Venue::Pacifica); // NOT flipped
        assert_eq!(eff.short_venue, Venue::Backpack);
    }

    #[test]
    fn mid_cycle_rebalance_to_different_pair_is_blocked() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        // Different pair, same canonical h (Pac<HL also gives +1)
        let rebal = make_decision(Venue::Pacifica, Venue::Hyperliquid, 100.0);
        let out = reg.enforce_decision("BTC", 1_700_000_500.0, Some(&rebal), false);
        assert!(
            out.pair_flip_blocked,
            "pair rebalance must be blocked even with matching h_c"
        );
        let eff = out.effective.unwrap();
        assert_eq!(eff.short_venue, Venue::Backpack); // still old pair
    }

    #[test]
    fn new_cycle_after_boundary_accepts_flip() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        // Cross the 3600s boundary
        let flip = make_decision(Venue::Backpack, Venue::Pacifica, 150.0);
        let out = reg.enforce_decision(
            "BTC",
            1_700_000_000.0 + DEFAULT_CYCLE_SECONDS as f64 + 10.0,
            Some(&flip),
            false,
        );
        assert!(out.opened_new_cycle);
        let eff = out.effective.unwrap();
        assert_eq!(eff.long_venue, Venue::Backpack); // flipped, new cycle
        assert!((eff.notional_usd - 150.0).abs() < 1e-12);
    }

    #[test]
    fn no_proposal_holds_through_gap() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        let out = reg.enforce_decision("BTC", 1_700_000_500.0, None, false);
        assert!(
            out.effective.is_some(),
            "should carry locked decision through the silent tick"
        );
    }

    #[test]
    fn emergency_override_passes_through() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        let flip = make_decision(Venue::Backpack, Venue::Pacifica, 0.0);
        let out = reg.enforce_decision("BTC", 1_700_000_500.0, Some(&flip), true);
        assert!(out.raw.emergency_override_used);
        // Effective reflects proposed (override semantics)
        let eff = out.effective.unwrap();
        assert_eq!(eff.long_venue, Venue::Backpack);
    }
}
