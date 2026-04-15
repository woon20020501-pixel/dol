//! Rust port of `strategy/funding_cycle_lock.py` — the Iron Law §1 gate.
//!
//! This is Wall 1 of the 4 safety walls (`integration-spec.md` §4). Every
//! order with a non-zero direction MUST trace to a call of [`enforce`] on
//! the same tick, and the order's direction MUST equal the returned
//! `h_eff`. Flipping direction without `emergency_override=true` is
//! forbidden.
//!
//! Ported for the Week 1 hackathon demo so that the decision log path
//! already exercises `enforce()` — this way when live submission is wired
//! in Week 2+, there is no I-LOCK escape hatch to plug afterwards.
//!
//! Parity target: `strategy/funding_cycle_lock.py` (aurora-omega-1.1.3).
//! Numerical parity is not applicable — this module is pure control-flow
//! logic over integer cycle indices.

use serde::{Deserialize, Serialize};

/// Default Pacifica funding cycle cadence in seconds.
pub const DEFAULT_CYCLE_SECONDS: i64 = 3600;

/// Valid direction values: -1 (short), 0 (flat), +1 (long funding leg).
/// Any other value is a programmer error.
pub fn is_valid_direction(h: i8) -> bool {
    matches!(h, -1..=1)
}

/// Immutable snapshot of the current locked cycle.
///
/// Mirrors the Python `CycleState` dataclass.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CycleState {
    /// `floor(opened_at / cycle_seconds)` at the moment of opening.
    pub cycle_index: i64,
    /// Locked funding-leg direction in `{-1, 0, +1}`.
    pub h_c: i8,
    /// Locked target notional in USD (≥ 0).
    pub n_c: f64,
    /// Unix seconds when the cycle was opened.
    pub opened_at: f64,
    /// Cadence this cycle was opened under (stored to avoid mixed-cadence
    /// readback bugs when a non-default cadence is used).
    pub cycle_seconds: i64,
}

/// Return the integer funding cycle index for a unix time.
///
/// Python: `int(t // cycle_seconds)`. Rust: `t.div_euclid(cycle_seconds)`.
pub fn cycle_index(t: f64, cycle_seconds: i64) -> i64 {
    assert!(cycle_seconds > 0, "cycle_seconds must be positive");
    (t / cycle_seconds as f64).floor() as i64
}

/// Fraction of the current cycle that has elapsed (0.0 ..= 1.0).
pub fn cycle_phase(t: f64, cycle_seconds: i64) -> f64 {
    assert!(cycle_seconds > 0, "cycle_seconds must be positive");
    (t.rem_euclid(cycle_seconds as f64)) / cycle_seconds as f64
}

/// Seconds remaining until the next cycle boundary.
pub fn seconds_to_cycle_end(t: f64, cycle_seconds: i64) -> f64 {
    assert!(cycle_seconds > 0, "cycle_seconds must be positive");
    cycle_seconds as f64 - t.rem_euclid(cycle_seconds as f64)
}

/// True iff there is an open cycle and we are still inside it.
///
/// If `state` is `None`, returns `false`. If `now` belongs to a later cycle
/// index, the lock has expired — the caller must close the old state and
/// open a new one via [`open_cycle`].
pub fn is_locked(state: Option<&CycleState>, now: f64) -> bool {
    match state {
        None => false,
        Some(s) => cycle_index(now, s.cycle_seconds) == s.cycle_index,
    }
}

/// Open a new locked cycle at time `now` with direction `h_c` and notional `n_c`.
///
/// # Panics
/// If `h_c` is not in `{-1, 0, +1}` or `n_c` is negative.
pub fn open_cycle(now: f64, h_c: i8, n_c: f64, cycle_seconds: i64) -> CycleState {
    assert!(
        is_valid_direction(h_c),
        "h_c must be in {{-1, 0, 1}}, got {h_c}"
    );
    assert!(n_c >= 0.0, "n_c must be non-negative, got {n_c}");
    CycleState {
        cycle_index: cycle_index(now, cycle_seconds),
        h_c,
        n_c,
        opened_at: now,
        cycle_seconds,
    }
}

/// Result of an [`enforce`] call — what the caller actually gets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EnforceResult {
    /// Effective direction — caller MUST use this, not the proposed value.
    pub h_eff: i8,
    /// Effective notional — caller MUST use this, not the proposed value.
    pub n_eff: f64,
    /// True if the proposed (h, N) differed from the effective (h, N) and
    /// the discrepancy was NOT resolved by opening a new cycle. Useful for
    /// telemetry and the "would_violate_lock" dashboard metric.
    pub proposed_was_blocked: bool,
    /// True if this call opened a new cycle (either first-time or after the
    /// previous cycle expired).
    pub opened_new_cycle: bool,
    /// True if `emergency_override=true` was passed AND the override
    /// actually bypassed an active lock (i.e., without the override the
    /// result would have differed).
    pub emergency_override_used: bool,
}

/// Apply the cycle lock to a proposed `(direction, notional)` at time `now`.
///
/// **Mutates** `state` in place: if no cycle is active or the previous cycle
/// expired, this call opens a new cycle at the proposed `(h, N)`. If a cycle
/// is active, the proposed values are ignored and the locked values are
/// returned.
///
/// # Priority order
///
/// 1. `emergency_override = true` → proposed values pass through, and if
///    there was an active lock the call marks `emergency_override_used=true`.
///    The caller MUST log every override at WARN.
/// 2. No lock (state is `None` or cycle expired) → open a new cycle at
///    `(proposed_h, proposed_n)` and return those. `opened_new_cycle=true`.
/// 3. Lock active → return the locked `(h_c, n_c)`. Proposed is ignored.
///    If proposed ≠ locked, `proposed_was_blocked=true`.
///
/// # Panics
/// If `proposed_h` is not in `{-1, 0, +1}` or `proposed_n` is negative.
pub fn enforce(
    state: &mut Option<CycleState>,
    now: f64,
    proposed_h: i8,
    proposed_n: f64,
    emergency_override: bool,
    cycle_seconds: i64,
) -> EnforceResult {
    assert!(
        is_valid_direction(proposed_h),
        "proposed_h must be in {{-1, 0, 1}}, got {proposed_h}"
    );
    assert!(
        proposed_n >= 0.0,
        "proposed_n must be non-negative, got {proposed_n}"
    );

    // Priority 1: emergency override
    if emergency_override {
        let was_locked = is_locked(state.as_ref(), now);
        // Override does NOT automatically reset the cycle state — the
        // caller is expected to decide whether to keep the lock or clear it
        // via a separate code path (typically the FSM emergency_flatten).
        // Python version has the same semantics.
        return EnforceResult {
            h_eff: proposed_h,
            n_eff: proposed_n,
            proposed_was_blocked: false,
            opened_new_cycle: false,
            emergency_override_used: was_locked
                && state
                    .as_ref()
                    .is_some_and(|s| s.h_c != proposed_h || s.n_c != proposed_n),
        };
    }

    // Priority 2: no active lock → open new cycle
    if !is_locked(state.as_ref(), now) {
        let new_state = open_cycle(now, proposed_h, proposed_n, cycle_seconds);
        *state = Some(new_state);
        return EnforceResult {
            h_eff: proposed_h,
            n_eff: proposed_n,
            proposed_was_blocked: false,
            opened_new_cycle: true,
            emergency_override_used: false,
        };
    }

    // Priority 3: active lock → locked values win
    let s = state.as_ref().expect("is_locked true ⇒ state is Some");
    let proposed_differs = s.h_c != proposed_h || s.n_c != proposed_n;
    EnforceResult {
        h_eff: s.h_c,
        n_eff: s.n_c,
        proposed_was_blocked: proposed_differs,
        opened_new_cycle: false,
        emergency_override_used: false,
    }
}

/// Lightweight check: `true` if a direction flip would be blocked by the
/// lock right now. Used for telemetry / logging / future RL reward shaping.
///
/// A "flip" is any transition from the locked `h_c` to a different value
/// (including `+1 → 0` and `0 → +1`).
pub fn would_violate_lock(state: Option<&CycleState>, now: f64, proposed_h: i8) -> bool {
    if !is_locked(state, now) {
        return false;
    }
    let s = state.expect("is_locked true ⇒ state is Some");
    proposed_h != s.h_c
}

// ═════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    const T0: f64 = 1_700_000_000.0; // some unix time in 2023
    const CYCLE: i64 = DEFAULT_CYCLE_SECONDS;

    #[test]
    fn cycle_index_floors() {
        assert_eq!(cycle_index(T0, CYCLE), (T0 as i64) / CYCLE);
        assert_eq!(cycle_index(T0 + 1.0, CYCLE), (T0 as i64) / CYCLE);
        assert_eq!(
            cycle_index(T0 + CYCLE as f64, CYCLE),
            1 + (T0 as i64) / CYCLE
        );
    }

    #[test]
    fn seconds_to_end_counts_down() {
        let t_start = (T0 as i64 / CYCLE) as f64 * CYCLE as f64;
        assert!((seconds_to_cycle_end(t_start + 0.0, CYCLE) - CYCLE as f64).abs() < 1e-9);
        assert!(
            (seconds_to_cycle_end(t_start + 1000.0, CYCLE) - (CYCLE as f64 - 1000.0)).abs() < 1e-9
        );
    }

    #[test]
    fn is_locked_none_state_is_false() {
        assert!(!is_locked(None, T0));
    }

    #[test]
    fn open_cycle_records_index() {
        // Align to a cycle boundary so "T0 + CYCLE - 1" stays in the same cycle.
        let aligned_t = ((T0 as i64) / CYCLE * CYCLE) as f64;
        let s = open_cycle(aligned_t, 1, 100.0, CYCLE);
        assert_eq!(s.h_c, 1);
        assert!((s.n_c - 100.0).abs() < 1e-12);
        assert_eq!(s.cycle_index, cycle_index(aligned_t, CYCLE));
        assert!(is_locked(Some(&s), aligned_t));
        assert!(is_locked(Some(&s), aligned_t + CYCLE as f64 - 1.0));
        assert!(!is_locked(Some(&s), aligned_t + CYCLE as f64 + 1.0));
    }

    #[test]
    fn enforce_opens_cycle_when_none() {
        let mut state = None;
        let r = enforce(&mut state, T0, 1, 100.0, false, CYCLE);
        assert_eq!(r.h_eff, 1);
        assert!((r.n_eff - 100.0).abs() < 1e-12);
        assert!(r.opened_new_cycle);
        assert!(!r.proposed_was_blocked);
        assert!(state.is_some());
    }

    #[test]
    fn enforce_holds_locked_direction() {
        let mut state = Some(open_cycle(T0, 1, 100.0, CYCLE));
        // Attempt to flip mid-cycle
        let r = enforce(&mut state, T0 + 500.0, -1, 200.0, false, CYCLE);
        assert_eq!(r.h_eff, 1);
        assert!((r.n_eff - 100.0).abs() < 1e-12);
        assert!(!r.opened_new_cycle);
        assert!(r.proposed_was_blocked);
    }

    #[test]
    fn enforce_reopens_after_cycle_boundary() {
        let mut state = Some(open_cycle(T0, 1, 100.0, CYCLE));
        let later = T0 + CYCLE as f64 + 10.0;
        let r = enforce(&mut state, later, -1, 200.0, false, CYCLE);
        assert_eq!(r.h_eff, -1);
        assert!((r.n_eff - 200.0).abs() < 1e-12);
        assert!(r.opened_new_cycle);
        assert!(!r.proposed_was_blocked);
        // State now carries the new lock.
        assert_eq!(state.as_ref().unwrap().h_c, -1);
    }

    #[test]
    fn enforce_emergency_override_passes_through() {
        let mut state = Some(open_cycle(T0, 1, 100.0, CYCLE));
        let r = enforce(&mut state, T0 + 500.0, -1, 0.0, true, CYCLE);
        assert_eq!(r.h_eff, -1);
        assert!((r.n_eff - 0.0).abs() < 1e-12);
        assert!(r.emergency_override_used);
        // Override does NOT clear the stored state — that's a separate code path.
        assert!(state.is_some());
    }

    #[test]
    fn would_violate_lock_detects_flip() {
        let s = open_cycle(T0, 1, 100.0, CYCLE);
        assert!(!would_violate_lock(Some(&s), T0 + 100.0, 1)); // same dir
        assert!(would_violate_lock(Some(&s), T0 + 100.0, -1)); // flip
        assert!(would_violate_lock(Some(&s), T0 + 100.0, 0)); // to flat
    }

    #[test]
    fn would_violate_lock_false_when_no_state() {
        assert!(!would_violate_lock(None, T0, 1));
    }

    #[test]
    #[should_panic(expected = "must be in")]
    fn enforce_rejects_invalid_direction() {
        let mut state = None;
        enforce(&mut state, T0, 5, 100.0, false, CYCLE);
    }

    #[test]
    #[should_panic(expected = "must be non-negative")]
    fn enforce_rejects_negative_notional() {
        let mut state = None;
        enforce(&mut state, T0, 1, -10.0, false, CYCLE);
    }
}
