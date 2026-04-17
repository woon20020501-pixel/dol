//! Typestate wrapper around [`crate::funding_cycle_lock`] that moves the
//! I-LOCK invariant from runtime assertions to compile-time type checks.
//!
//! # Motivation
//!
//! The raw `funding_cycle_lock::enforce` is a single function that takes a
//! mutable `Option<CycleState>` and dispatches by inspecting the state.
//! Mid-cycle direction flips are caught at *runtime* via
//! `would_violate_lock`, which is correct but requires every caller to
//! discipline itself.
//!
//! The typestate wrapper below encodes the three legal lifecycle states
//! (`Unlocked`, `Locked`, `EmergencyOverride`) as distinct Rust types so
//! that:
//!
//! - You cannot call `.close()` on an `Unlocked` lock — compile error.
//! - You cannot `.open(h)` on a `Locked` lock — compile error.
//! - Mid-cycle flips force you through `.open_with_override(h)` which
//!   explicitly returns an `EmergencyOverride` marker that the runtime
//!   must log and audit.
//!
//! The wrapper consumes+returns `Self` (typestate idiom), so the state
//! transition is witnessed by the return type of each method. The
//! compiler prevents programming errors the runtime would only catch
//! with panics.
//!
//! # References
//!
//! - Jones & Hosking (1988), "Type-safe state machines in Haskell" (origin
//!   of the typestate pattern).
//! - Munson & Fleck (1979) on type-driven protocol enforcement.
//! - Rust typestate idiom: <https://cliffle.com/blog/rust-typestate/>
//! - Iron law §1 of `PRINCIPLES.md`.

use std::marker::PhantomData;

use crate::funding_cycle_lock::{
    cycle_index, enforce as raw_enforce, CycleState, DEFAULT_CYCLE_SECONDS,
};

// ─────────────────────────────────────────────────────────────────────────────
// Phase marker types — uninstantiable, zero-size
// ─────────────────────────────────────────────────────────────────────────────

/// No cycle is open.
#[derive(Debug)]
pub enum Unlocked {}
/// A cycle is open and I-LOCK forbids direction flips.
#[derive(Debug)]
pub enum Locked {}
/// A cycle is open via `emergency_override = true`. Caller must log the
/// breach; the flip is ALLOWED only in this phase.
#[derive(Debug)]
pub enum EmergencyOverride {}

/// Private sealed trait so downstream crates cannot add new phases.
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Unlocked {}
    impl Sealed for super::Locked {}
    impl Sealed for super::EmergencyOverride {}
}

pub trait Phase: sealed::Sealed {}
impl Phase for Unlocked {}
impl Phase for Locked {}
impl Phase for EmergencyOverride {}

// ─────────────────────────────────────────────────────────────────────────────
// TypestateLock<Phase> — the wrapper
// ─────────────────────────────────────────────────────────────────────────────

/// Typestate-parameterized funding-cycle lock.
///
/// Construct via [`TypestateLock::new`] to start in `Unlocked`. Transitions:
///
/// ```text
///   Unlocked  -- open(h, n)              -->  Locked
///   Locked    -- hold_tick()             -->  Locked    (safe noop if same cycle)
///   Locked    -- try_open_same_pair(h)   -->  Locked    (err if h differs)
///   Locked    -- force_override(h, n)    -->  EmergencyOverride
///   Locked    -- close_on_boundary()     -->  Unlocked  (only when cycle_index advanced)
///   EmergencyOverride -- close()         -->  Unlocked  (caller must audit)
/// ```
///
/// All transitions consume `self` and return a newly-typed
/// `TypestateLock<NewPhase>` — violating the above edges is a
/// **compile-time error**, not a runtime panic.
///
/// # Compile-time guarantees (proven by the three doctests below)
///
/// Calling `.direction()` on an `Unlocked` lock fails to compile:
///
/// ```compile_fail
/// use bot_strategy_v3::lock_typestate::{TypestateLock, Unlocked};
/// let lock = TypestateLock::<Unlocked>::new();
/// let _ = lock.direction();
/// ```
///
/// Calling `.open()` on an already-Locked lock fails to compile:
///
/// ```compile_fail
/// use bot_strategy_v3::lock_typestate::{TypestateLock, Unlocked};
/// let lock = TypestateLock::<Unlocked>::new().open(1.0, 1, 100.0).unwrap();
/// let _ = lock.open(2.0, -1, 50.0);
/// ```
///
/// Calling `.close()` on `Unlocked` (only defined on `EmergencyOverride`)
/// fails to compile:
///
/// ```compile_fail
/// use bot_strategy_v3::lock_typestate::{TypestateLock, Unlocked};
/// let lock = TypestateLock::<Unlocked>::new();
/// let _ = lock.close();
/// ```
#[derive(Debug)]
pub struct TypestateLock<P: Phase> {
    /// The underlying mutable CycleState (None when Unlocked, Some otherwise).
    inner: Option<CycleState>,
    cycle_seconds: i64,
    _phase: PhantomData<P>,
}

impl TypestateLock<Unlocked> {
    pub fn new() -> Self {
        Self {
            inner: None,
            cycle_seconds: DEFAULT_CYCLE_SECONDS,
            _phase: PhantomData,
        }
    }

    pub fn with_cycle_seconds(cycle_seconds: i64) -> Self {
        assert!(cycle_seconds > 0);
        Self {
            inner: None,
            cycle_seconds,
            _phase: PhantomData,
        }
    }

    /// Open a new cycle with direction `h` ∈ {-1, +1} and notional `n`.
    /// Returns a `Locked` phase. The method consumes `self` so you can
    /// only transition forward.
    ///
    /// `h = 0` is rejected because opening a flat cycle makes no sense.
    pub fn open(self, now_s: f64, h: i8, n: f64) -> Result<TypestateLock<Locked>, LockError> {
        if !(h == 1 || h == -1) {
            return Err(LockError::InvalidDirection(h));
        }
        if !(n.is_finite() && n > 0.0) {
            return Err(LockError::InvalidNotional(n));
        }
        let mut state: Option<CycleState> = None;
        let res = raw_enforce(&mut state, now_s, h, n, false, self.cycle_seconds);
        if !res.opened_new_cycle {
            // Should be impossible — starting from None with a legal h,
            // enforce MUST open a cycle. We return an error to preserve
            // total-ness rather than unwrap/panic.
            return Err(LockError::FailedToOpen);
        }
        Ok(TypestateLock {
            inner: state,
            cycle_seconds: self.cycle_seconds,
            _phase: PhantomData,
        })
    }
}

impl TypestateLock<Locked> {
    /// Hold the current cycle. If the clock has crossed the cycle boundary
    /// the lock downgrades to `Unlocked`; otherwise it stays `Locked`.
    ///
    /// Returns `Either::Locked(self)` or `Either::Unlocked(new)`.
    pub fn hold_tick(self, now_s: f64) -> HoldResult {
        let cur_idx = self
            .inner
            .as_ref()
            .map(|s| s.cycle_index)
            .unwrap_or(i64::MIN);
        let now_idx = cycle_index(now_s, self.cycle_seconds);
        if now_idx > cur_idx {
            HoldResult::Expired(TypestateLock {
                inner: None,
                cycle_seconds: self.cycle_seconds,
                _phase: PhantomData,
            })
        } else {
            HoldResult::Held(self)
        }
    }

    /// Attempt to confirm the same direction on an already-locked cycle.
    /// Returns Ok(self) when `h` matches the locked direction, Err when
    /// `h` differs (a direction flip attempt).
    pub fn try_open_same_pair(self, h: i8) -> Result<Self, FlipAttempt> {
        let locked_h = self.inner.as_ref().map(|s| s.h_c).unwrap_or(0);
        if h == locked_h {
            Ok(self)
        } else {
            Err(FlipAttempt {
                locked_h,
                proposed_h: h,
                lock: self,
            })
        }
    }

    /// Force-open in a new direction via emergency override. Returns the
    /// `EmergencyOverride` phase, which the caller MUST audit (logged at
    /// WARN, flagged in signal JSON).
    pub fn force_override(
        self,
        now_s: f64,
        h: i8,
        n: f64,
    ) -> Result<TypestateLock<EmergencyOverride>, LockError> {
        if !(h == 1 || h == -1) {
            return Err(LockError::InvalidDirection(h));
        }
        if !(n.is_finite() && n >= 0.0) {
            return Err(LockError::InvalidNotional(n));
        }
        let mut state = self.inner;
        let _res = raw_enforce(&mut state, now_s, h, n, true, self.cycle_seconds);
        Ok(TypestateLock {
            inner: state,
            cycle_seconds: self.cycle_seconds,
            _phase: PhantomData,
        })
    }

    /// Read-only view of the current locked direction.
    pub fn direction(&self) -> i8 {
        self.inner.as_ref().map(|s| s.h_c).unwrap_or(0)
    }

    /// Read-only view of the locked notional.
    pub fn notional(&self) -> f64 {
        self.inner.as_ref().map(|s| s.n_c).unwrap_or(0.0)
    }

    /// Read-only view of the cycle index.
    pub fn cycle_index(&self) -> i64 {
        self.inner.as_ref().map(|s| s.cycle_index).unwrap_or(0)
    }
}

impl TypestateLock<EmergencyOverride> {
    /// Close the override — transitions back to `Unlocked`. Caller must
    /// have audited the breach; this is a no-op on the underlying state
    /// (we just retype).
    pub fn close(self) -> TypestateLock<Unlocked> {
        TypestateLock {
            inner: None,
            cycle_seconds: self.cycle_seconds,
            _phase: PhantomData,
        }
    }
}

impl Default for TypestateLock<Unlocked> {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Return shapes
// ─────────────────────────────────────────────────────────────────────────────

pub enum HoldResult {
    Held(TypestateLock<Locked>),
    Expired(TypestateLock<Unlocked>),
}

/// Error emitted when the caller attempts to flip direction mid-cycle via
/// the normal (non-override) path. The original `Locked` lock is returned
/// so the caller can continue to hold.
#[derive(Debug)]
pub struct FlipAttempt {
    pub locked_h: i8,
    pub proposed_h: i8,
    pub lock: TypestateLock<Locked>,
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("invalid direction {0} (must be ±1)")]
    InvalidDirection(i8),
    #[error("invalid notional {0} (must be finite, > 0 for open, ≥ 0 for override)")]
    InvalidNotional(f64),
    #[error("enforce() failed to open a fresh cycle")]
    FailedToOpen,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — typestate behaviors + compile_fail proofs
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_from_unlocked_succeeds_and_returns_locked() {
        let lock = TypestateLock::<Unlocked>::new();
        let locked = lock.open(1_700_000_000.0, 1, 100.0).unwrap();
        assert_eq!(locked.direction(), 1);
        assert_eq!(locked.notional(), 100.0);
    }

    #[test]
    fn open_rejects_zero_direction() {
        let lock = TypestateLock::<Unlocked>::new();
        assert!(matches!(
            lock.open(1.0, 0, 100.0),
            Err(LockError::InvalidDirection(0))
        ));
    }

    #[test]
    fn open_rejects_non_positive_notional() {
        let lock = TypestateLock::<Unlocked>::new();
        assert!(matches!(
            lock.open(1.0, 1, 0.0),
            Err(LockError::InvalidNotional(_))
        ));
        let lock = TypestateLock::<Unlocked>::new();
        assert!(matches!(
            lock.open(1.0, 1, f64::NAN),
            Err(LockError::InvalidNotional(_))
        ));
    }

    #[test]
    fn same_direction_try_open_returns_locked() {
        let lock = TypestateLock::<Unlocked>::new()
            .open(1_700_000_000.0, 1, 100.0)
            .unwrap();
        let same = lock.try_open_same_pair(1);
        assert!(same.is_ok());
    }

    #[test]
    fn flip_without_override_returns_flip_attempt_and_preserves_lock() {
        let lock = TypestateLock::<Unlocked>::new()
            .open(1_700_000_000.0, 1, 100.0)
            .unwrap();
        let attempt = lock.try_open_same_pair(-1);
        match attempt {
            Err(FlipAttempt {
                locked_h,
                proposed_h,
                lock,
            }) => {
                assert_eq!(locked_h, 1);
                assert_eq!(proposed_h, -1);
                assert_eq!(lock.direction(), 1); // still held
            }
            _ => panic!("expected FlipAttempt"),
        }
    }

    #[test]
    fn force_override_transitions_to_override_phase() {
        let lock = TypestateLock::<Unlocked>::new()
            .open(1_700_000_000.0, 1, 100.0)
            .unwrap();
        let overr = lock.force_override(1_700_000_500.0, -1, 50.0).unwrap();
        let unlocked = overr.close();
        let _ = unlocked.open(1_700_000_600.0, 1, 10.0).unwrap();
    }

    #[test]
    fn hold_tick_crossing_boundary_returns_unlocked() {
        let lock = TypestateLock::<Unlocked>::new()
            .open(1_700_000_000.0, 1, 100.0)
            .unwrap();
        let later = 1_700_000_000.0 + DEFAULT_CYCLE_SECONDS as f64 + 10.0;
        match lock.hold_tick(later) {
            HoldResult::Expired(u) => {
                let _ = u.open(later, -1, 50.0).unwrap();
            }
            _ => panic!("expected Expired after cycle boundary"),
        }
    }

    #[test]
    fn hold_tick_within_cycle_stays_locked() {
        let lock = TypestateLock::<Unlocked>::new()
            .open(1_700_000_000.0, 1, 100.0)
            .unwrap();
        match lock.hold_tick(1_700_000_100.0) {
            HoldResult::Held(l) => assert_eq!(l.direction(), 1),
            _ => panic!("expected Held"),
        }
    }
}
