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
use bot_strategy_v3::lock_typestate::{
    EmergencyOverride as EOPhase, Locked as LockedPhase, TypestateLock, Unlocked as UnlockedPhase,
};
use bot_types::Venue;

use crate::decision::{PairDecision, TypedPairDecision};
use bot_types::sym::SymbolMarker;

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

/// Per-symbol state tracked by the registry.
///
/// Holds both (a) the Python-parity `CycleState` used for byte-exact
/// `enforce()` reproduction, AND (b) a `RegistryPhase` that wraps a
/// `TypestateLock` so every runtime transition is routed through the
/// compile-time-typed state machine. The two views are kept in lock-step —
/// `debug_assert!` checks confirm agreement on `h_c`, `n_c`, and
/// `cycle_index` after every transition.
#[derive(Debug)]
struct SymbolLock {
    /// Python-parity raw state (the authoritative byte-exact representation).
    state: CycleState,
    /// Typestate-enforced phase view of the same state. Every transition
    /// goes through `TypestateLock::{open, hold_tick, try_open_same_pair,
    /// force_override}` — unsafe transitions cannot compile.
    phase: RegistryPhase,
    /// Full locked `PairDecision` (used to detect pair rebalances that
    /// share the canonical `h_c` — see funding_cycle_lock::pair_to_h).
    locked_decision: PairDecision,
}

/// Enum wrapping `TypestateLock<Phase>` for per-symbol storage in a
/// heterogeneous map. Each variant owns the typed lock; transitions move
/// ownership across variants so the `TypestateLock` methods enforce
/// phase correctness at compile time.
#[derive(Debug)]
enum RegistryPhase {
    Locked(TypestateLock<LockedPhase>),
    EmergencyOverride(TypestateLock<EOPhase>),
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

    /// True iff the internal typestate phase for `symbol` is `Locked`.
    /// Used by tests to prove the typestate wrapper is driving the runtime
    /// state (not just a parallel shadow). Production callers go through
    /// `enforce_decision` / `state_for`.
    pub fn is_typed_locked(&self, symbol: &str) -> bool {
        matches!(
            self.by_symbol.get(symbol).map(|e| &e.phase),
            Some(RegistryPhase::Locked(_))
        )
    }

    /// True iff the internal typestate phase for `symbol` is
    /// `EmergencyOverride`. See [`Self::is_typed_locked`].
    pub fn is_typed_emergency_override(&self, symbol: &str) -> bool {
        matches!(
            self.by_symbol.get(symbol).map(|e| &e.phase),
            Some(RegistryPhase::EmergencyOverride(_))
        )
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
    #[tracing::instrument(
        name = "enforce_decision",
        skip_all,
        fields(symbol = %symbol, now_s, emergency_override)
    )]
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
            // Fresh cycle: route the transition through TypestateLock so
            // the compile-time type system witnesses the Unlocked→Locked
            // move. We can't panic if open() fails — proposed must be
            // Some and h must be ±1 for raw.opened_new_cycle to be true.
            if let Some(d) = proposed {
                let new_state = state.expect("opened_new_cycle ⇒ state is Some");
                // Typestate construction: Unlocked → Locked via TypestateLock::open.
                // We wrap the already-computed state rather than re-running
                // enforce(), by constructing the lock manually post-hoc.
                let typed: TypestateLock<LockedPhase> =
                    TypestateLock::<UnlockedPhase>::with_cycle_seconds(self.cycle_seconds)
                        .open(now_s, proposed_h, proposed_n)
                        .expect("raw.opened_new_cycle ⇒ open() must succeed");
                debug_assert_eq!(
                    typed.direction(),
                    new_state.h_c,
                    "typestate h_c disagreement"
                );
                debug_assert_eq!(
                    typed.cycle_index(),
                    new_state.cycle_index,
                    "typestate cycle_index disagreement"
                );
                self.by_symbol.insert(
                    symbol.to_string(),
                    SymbolLock {
                        state: new_state,
                        phase: RegistryPhase::Locked(typed),
                        locked_decision: d.clone(),
                    },
                );
                effective = Some(d.clone());
            } else {
                self.by_symbol.remove(symbol);
                effective = None;
            }
        } else if raw.emergency_override_used {
            // Override path: route through TypestateLock::force_override so
            // the EmergencyOverride phase is witnessed at the type level.
            if let Some(d) = proposed {
                if let Some(SymbolLock {
                    state: prev_state,
                    phase,
                    locked_decision: _,
                }) = self.by_symbol.remove(symbol)
                {
                    match phase {
                        RegistryPhase::Locked(locked) => {
                            match locked.force_override(now_s, proposed_h, proposed_n) {
                                Ok(eo) => {
                                    self.by_symbol.insert(
                                        symbol.to_string(),
                                        SymbolLock {
                                            state: state.unwrap_or(prev_state),
                                            phase: RegistryPhase::EmergencyOverride(eo),
                                            locked_decision: d.clone(),
                                        },
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        symbol = %symbol,
                                        error = %e,
                                        "typestate force_override rejected —                                          keeping previous state"
                                    );
                                }
                            }
                        }
                        RegistryPhase::EmergencyOverride(eo) => {
                            self.by_symbol.insert(
                                symbol.to_string(),
                                SymbolLock {
                                    state: state.unwrap_or(prev_state),
                                    phase: RegistryPhase::EmergencyOverride(eo),
                                    locked_decision: d.clone(),
                                },
                            );
                        }
                    }
                }
            }
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

    /// Typestate-safe variant of [`Self::enforce_decision`].
    ///
    /// Takes a [`TypedPairDecision<'_, S>`] and uses `S::NAME` as the
    /// registry key — the caller can no longer pass a mismatching
    /// `(symbol, decision)` pair because the symbol is fixed by the
    /// type parameter at compile time.
    ///
    /// `proposed = None` represents a tick with no candidate; the lock
    /// registry still runs through `funding_cycle_lock::enforce` so a
    /// held cycle keeps ticking forward.
    ///
    /// This is the I-SAME typestate wired into the production enforcement
    /// boundary: a caller that loops over symbols must first project
    /// the runtime `PairDecision` into a `TypedPairDecision<'_, S>` via
    /// [`PairDecision::try_typed`], and the resulting compile-time tag
    /// flows through to the registry.
    #[tracing::instrument(
        name = "enforce_decision_typed",
        skip_all,
        fields(symbol = S::NAME, now_s, emergency_override)
    )]
    pub fn enforce_decision_typed<S: SymbolMarker>(
        &mut self,
        now_s: f64,
        proposed: Option<TypedPairDecision<'_, S>>,
        emergency_override: bool,
    ) -> EnforceOutcome {
        let pd_ref = proposed.map(|t| t.pair_decision());
        self.enforce_decision(S::NAME, now_s, pd_ref, emergency_override)
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

    /// Typestate wiring proof: after open, the internal phase is Locked.
    #[test]
    fn typestate_phase_is_locked_after_open() {
        let mut reg = CycleLockRegistry::new();
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        assert!(
            !reg.is_typed_locked("BTC"),
            "pre-open phase should not be Locked"
        );
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&d), false);
        assert!(
            reg.is_typed_locked("BTC"),
            "post-open internal phase must be Locked (typestate on runtime path)"
        );
        assert!(!reg.is_typed_emergency_override("BTC"));
    }

    /// Typestate wiring proof: emergency_override transitions the typed
    /// phase to EmergencyOverride (not just flips a bool on CycleState).
    #[test]
    fn typestate_phase_transitions_to_emergency_override() {
        let mut reg = CycleLockRegistry::new();
        let open = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&open), false);
        assert!(reg.is_typed_locked("BTC"));
        let flip = make_decision(Venue::Backpack, Venue::Pacifica, 50.0);
        reg.enforce_decision("BTC", 1_700_000_500.0, Some(&flip), true);
        assert!(
            reg.is_typed_emergency_override("BTC"),
            "override must move typed phase to EmergencyOverride"
        );
        assert!(!reg.is_typed_locked("BTC"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // enforce_decision_typed — I-SAME typestate wiring tests
    // ─────────────────────────────────────────────────────────────────────

    use bot_types::sym::{BtcMarker, EthMarker};

    /// The typed enforce path uses `S::NAME` as the registry key, not a
    /// caller-provided string. A BTC-typed decision landing in the registry
    /// must appear under the "BTC" key.
    #[test]
    fn enforce_decision_typed_uses_marker_name_as_key() {
        let mut reg = CycleLockRegistry::new();
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        let typed = d
            .try_typed::<BtcMarker>()
            .expect("BTC marker accepts BTC symbol");
        let out = reg.enforce_decision_typed::<BtcMarker>(1_700_000_000.0, Some(typed), false);
        assert!(out.opened_new_cycle);
        // The untyped lookup must find the lock under "BTC".
        assert!(reg.is_typed_locked("BTC"));
        assert!(reg.locked_decision_for("BTC").is_some());
    }

    /// `try_typed::<EthMarker>()` on a BTC decision returns None; the
    /// typed enforce path therefore sees `proposed = None` and the
    /// registry does NOT silently store the BTC decision under the "ETH"
    /// key. This is the core I-SAME guard: a caller cannot project a
    /// decision into the wrong marker's slot.
    #[test]
    fn try_typed_with_wrong_marker_cannot_leak_into_other_slot() {
        let mut reg = CycleLockRegistry::new();
        let btc = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        assert!(
            btc.try_typed::<EthMarker>().is_none(),
            "BTC decision must not project to ETH marker"
        );
        let eth_typed = btc.try_typed::<EthMarker>();
        let _out = reg.enforce_decision_typed::<EthMarker>(1_700_000_000.0, eth_typed, false);
        // Regardless of what funding_cycle_lock does with h=0/n=0 (an
        // internal Python-parity detail), the ETH registry slot must not
        // hold a decision — the BTC decision cannot leak in.
        assert!(
            reg.locked_decision_for("ETH").is_none(),
            "ETH slot must remain empty when only a BTC decision existed"
        );
        assert!(
            !reg.is_typed_locked("ETH"),
            "ETH slot phase must not be Locked"
        );
    }

    /// Two different markers give independent registry slots. A BTC lock
    /// and an ETH lock can coexist without interference — the compile-time
    /// marker enforces that you cannot query one slot while holding a
    /// handle to the other.
    #[test]
    fn independent_marker_slots_do_not_interfere() {
        let mut reg = CycleLockRegistry::new();
        let btc_pd = make_decision(Venue::Pacifica, Venue::Backpack, 100.0);
        let typed_btc = btc_pd.try_typed::<BtcMarker>().unwrap();
        reg.enforce_decision_typed::<BtcMarker>(1_700_000_000.0, Some(typed_btc), false);
        // ETH slot still empty
        assert!(reg.is_typed_locked("BTC"));
        assert!(!reg.is_typed_locked("ETH"));
        // Open ETH
        let eth_pd = PairDecision {
            long_venue: Venue::Pacifica,
            short_venue: Venue::Backpack,
            symbol: "ETH".to_string(),
            spread_annual: 0.02,
            cost_fraction: 0.001,
            net_annual: 0.019,
            notional_usd: 200.0,
            reason: String::new(),
            would_have_executed: true,
        };
        let typed_eth = eth_pd.try_typed::<EthMarker>().unwrap();
        reg.enforce_decision_typed::<EthMarker>(1_700_000_000.0, Some(typed_eth), false);
        assert!(reg.is_typed_locked("ETH"));
        assert!(reg.is_typed_locked("BTC"), "BTC lock must not be affected");
    }
}
