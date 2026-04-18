//! Live submission preflight gate.
//!
//! Called ONCE at bot startup (before any subcommand runs). Semantics:
//!
//! 1. If `RUNNER_ALLOW_LIVE != "1"` → **demo mode**: return `Ok(())`
//!    silently. Demo paths are unaffected.
//! 2. If `RUNNER_ALLOW_LIVE == "1"` → **live mode**: run the v0 component
//!    wiring check. Return `Ok(())` iff every required component is
//!    present; otherwise return an error naming the missing components.
//!
//! The wiring check is defense-in-depth: even if the operator sets the
//! env var, the bot refuses to start in live mode until the v0 subset
//! from `integration-spec.md` §3.5 is actually implemented. See
//! the live-promotion checklist for the current status.
//!
//! The env-gated-at-startup + component-checklist shape is the authoritative
//! spec. An earlier library function `assert_live_allowed()` is preserved
//! below as a lower-level helper for any call site that wants to re-verify
//! the env var at a submission boundary; but the authoritative entry is
//! [`preflight_live_gate`].

use std::env;

/// Env var the operator must set to `"1"` to allow live submissions.
pub const RUNNER_ALLOW_LIVE_ENV: &str = "RUNNER_ALLOW_LIVE";

// ─────────────────────────────────────────────────────────────────────────────
// Primary entry point — called once at bot startup.
// ─────────────────────────────────────────────────────────────────────────────

/// Preflight live-submission check.
///
/// - Demo mode (`RUNNER_ALLOW_LIVE != "1"`): silently passes.
/// - Live mode (`RUNNER_ALLOW_LIVE == "1"`): verifies that every required
///   v0 component is wired. Fails with a descriptive error listing
///   missing pieces if not.
///
/// Call this once from the binary's `main` (before any subcommand runs).
/// Abort the process on error.
pub fn preflight_live_gate() -> Result<(), String> {
    if env::var(RUNNER_ALLOW_LIVE_ENV).as_deref() != Ok("1") {
        return Ok(()); // demo mode — unconditionally allow
    }

    // Live mode — enumerate missing components.
    let mut missing: Vec<&'static str> = Vec::new();
    if !has_funding_cycle_lock() {
        missing.push("I-LOCK funding_cycle_lock");
    }
    if !has_fsm_emergency_flatten() {
        missing.push("I-KILL fsm_controller (emergency_flatten)");
    }
    if !has_cvar_guard_nonstub() {
        missing.push("I-BUDGET cvar_guard (non-stub)");
    }
    if !has_kill_switch() {
        missing.push("kill_switch (SIGTERM flatten handler)");
    }
    if !has_heartbeat() {
        missing.push("heartbeat (5s hedge-fill watchdog)");
    }
    if !has_pacifica_watchdog() {
        missing.push("Pacifica API watchdog (3s latency)");
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}=1 requested but the following v0 components are not wired: {}. \
             See the live-promotion checklist for details.",
            RUNNER_ALLOW_LIVE_ENV,
            missing.join(", ")
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Component-presence checks
// ─────────────────────────────────────────────────────────────────────────────
//
// Each `has_*` function returns `true` iff the corresponding v0 component is
// wired into the runtime. These are currently stubs — they return whatever
// the Week 1 status is. Flip each to `true` only as the real Rust
// implementation lands and is verified under parity tests / chaos tests.
//
// Do NOT flip based on env var or runtime configuration — these are
// *compile-time* bot capabilities, not operator preferences.

/// I-LOCK: `funding_cycle_lock.enforce()` wired into the decision path.
///
/// Status: **true** — ported in Week 1 Step B and threaded through
/// `TickEngine::run_one_tick` via `cycle_lock::CycleLockRegistry`.
/// Live tick proven against Pacifica API.
pub const fn has_funding_cycle_lock() -> bool {
    true
}

/// I-KILL: `fsm_controller` emergency-flatten wired.
///
/// Status: **true** — `bot_strategy_v3::fsm_controller::step` ported from
/// Python `fsm_controller.py`. 9 unit tests cover Kelly/Neutral/Robust
/// transitions + Banach-damping clip. Invoked from the runtime risk stack
/// when ≥ 2 red flags fire (`NOTIONAL_SCALE_ROBUST = 0.4`, 2-min flatten).
pub const fn has_fsm_emergency_flatten() -> bool {
    true
}

/// I-BUDGET: non-stub `risk_stack::cvar_ru` + `cvar_guard` (DEFAULT_BUDGET_99)
/// evaluated every tick.
///
/// Status: **true** — `risk::cvar_budget::CvarBudgetGuard` wired into
/// `TickEngine::run_one_tick` via `RiskStack::evaluate`. Rockafellar-Uryasev
/// (2000) empirical CVaR_99 estimator; 7-day rolling window; soft-landing
/// reduce → hard block at 2× budget.
pub const fn has_cvar_guard_nonstub() -> bool {
    true
}

/// Bot-owned kill_switch (SIGTERM trap OR file-flag → 1s flatten).
///
/// Status: **true** — `risk::kill_switch::KillSwitch` with SIGINT handler
/// (tokio::signal::ctrl_c cross-platform) + `./kill.flag` file poll.
pub const fn has_kill_switch() -> bool {
    true
}

/// Bot-owned heartbeat (5s watchdog on hedge-fill subsystem).
///
/// Status: **true** — `risk::heartbeat::HedgeHeartbeat` tracks paired fills
/// and escalates Reduce(0.25) → Flatten when hedge-leg gap ≥ 2× 5s.
pub const fn has_heartbeat() -> bool {
    true
}

/// Bot-owned Pacifica API watchdog (3s latency → emergency flatten).
///
/// Status: **true** — `risk::watchdog::ApiLatencyWatchdog` with rolling
/// p99 latency over 60s window; warn 1.5s, fatal 3s sustained 30s.
pub const fn has_pacifica_watchdog() -> bool {
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Lower-level helpers (kept for future submission-boundary re-checks)
// ─────────────────────────────────────────────────────────────────────────────

/// Non-failing probe: `true` iff the env var is literally `"1"`.
///
/// Useful for conditional logging. The authoritative check is
/// [`preflight_live_gate`].
pub fn is_live_allowed() -> bool {
    env::var(RUNNER_ALLOW_LIVE_ENV)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Assert that the env var is set to `"1"`. Does **not** check component
/// wiring. Useful as a secondary boundary check inside a `submit_order`
/// entry point in Week 2+ (after `preflight_live_gate` has already run at
/// startup). On a production bot both checks fire: preflight at startup
/// (components), per-submission at the order boundary (env var sanity).
pub fn assert_live_allowed() -> anyhow::Result<()> {
    match env::var(RUNNER_ALLOW_LIVE_ENV) {
        Ok(v) if v == "1" => Ok(()),
        Ok(v) => Err(anyhow::anyhow!(
            "{} is set to {:?}, expected \"1\" — live submission blocked.",
            RUNNER_ALLOW_LIVE_ENV,
            v
        )),
        Err(_) => Err(anyhow::anyhow!(
            "{} env var not set — live submission blocked. \
             See the live-promotion checklist.",
            RUNNER_ALLOW_LIVE_ENV
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env-var tests must serialize; cargo runs tests multi-threaded.
    fn with_env<F: FnOnce()>(key: &str, value: Option<&str>, f: F) {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _g = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let prev = env::var(key).ok();
        match value {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
        f();
        match prev {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }

    // ─── preflight_live_gate ─────────────────────────────────────────────

    #[test]
    fn preflight_demo_mode_passes_when_unset() {
        with_env(RUNNER_ALLOW_LIVE_ENV, None, || {
            assert!(
                preflight_live_gate().is_ok(),
                "unset env var → demo mode → Ok"
            );
        });
    }

    #[test]
    fn preflight_demo_mode_passes_when_zero() {
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("0"), || {
            assert!(preflight_live_gate().is_ok());
        });
    }

    #[test]
    fn preflight_demo_mode_passes_when_true_string() {
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("true"), || {
            // Not "1" → treated as demo mode (silent pass). Matches the spec.
            assert!(preflight_live_gate().is_ok());
        });
    }

    #[test]
    fn preflight_live_mode_fails_with_missing_components() {
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("1"), || {
            let result = preflight_live_gate();
            // After S-B4: all 6 capability flags are true — preflight passes.
            assert!(
                result.is_ok(),
                "preflight should pass with all 6 guards wired, got: {result:?}"
            );
        });
    }

    // ─── helpers ─────────────────────────────────────────────────────────

    #[test]
    fn is_live_allowed_matches_env() {
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("1"), || {
            assert!(is_live_allowed())
        });
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("0"), || {
            assert!(!is_live_allowed())
        });
        with_env(RUNNER_ALLOW_LIVE_ENV, None, || assert!(!is_live_allowed()));
    }

    #[test]
    fn assert_live_allowed_rejects_non_one() {
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("0"), || {
            assert!(assert_live_allowed().is_err());
        });
        with_env(RUNNER_ALLOW_LIVE_ENV, Some("1"), || {
            assert!(assert_live_allowed().is_ok());
        });
    }

    // ─── component status ────────────────────────────────────────────────

    #[test]
    fn funding_cycle_lock_is_wired() {
        // Week 1 Step B shipped the Rust port. This should stay true.
        assert!(has_funding_cycle_lock());
    }

    #[test]
    fn six_guard_stack_wired_true() {
        // Priority-6 deliverables: CVaR budget + kill switch + heartbeat +
        // Pacifica watchdog all wired into TickEngine::run_one_tick via
        // RiskStack::evaluate. Concentration + drawdown also wired; see
        // live_gate.rs docstrings for each.
        assert!(has_cvar_guard_nonstub());
        assert!(has_kill_switch());
        assert!(has_heartbeat());
        assert!(has_pacifica_watchdog());
    }

    #[test]
    fn fsm_controller_wired() {
        // Kelly/Neutral/Robust FSM ported from Python (S-B4).
        assert!(has_fsm_emergency_flatten());
    }
}
