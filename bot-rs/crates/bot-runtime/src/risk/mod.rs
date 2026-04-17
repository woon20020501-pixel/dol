//! Runtime risk-management stack.
//!
//! Six-guard defense (addresses `live_gate::has_cvar_guard_nonstub`,
//! `has_kill_switch`, `has_heartbeat`, `has_pacifica_watchdog`):
//!
//! 1. [`cvar_budget::CvarBudgetGuard`] — rolling-window CVaR_99 vs NAV budget.
//! 2. [`kill_switch::KillSwitch`]       — SIGINT + `./kill.flag` → flatten.
//! 3. [`heartbeat::HedgeHeartbeat`]     — 5s watchdog on hedge-leg fills.
//! 4. [`watchdog::ApiLatencyWatchdog`]  — 3s Pacifica REST latency alarm.
//! 5. [`concentration::VenueConcentrationCap`] — HHI ≤ 0.5 admission gate.
//! 6. [`drawdown::DrawdownStop`]        — cvar_drawdown_stop wired to NAV.
//!
//! Every guard exposes a pure `check(state) -> RiskDecision` API so the
//! decision engine composes them as a sequential filter chain.

pub mod concentration;
pub mod cvar_budget;
pub mod drawdown;
pub mod heartbeat;
pub mod kill_switch;
pub mod stack;
pub mod watchdog;

pub use stack::{build_exposures, RiskStack};

use serde::Serialize;

/// Output of a single risk guard. Guards that block include a human-readable
/// reason for operator visibility (logged at WARN, emitted in signal JSON).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum RiskDecision {
    /// Pass — the guard found no violation.
    Pass,
    /// Reduce size — downsize new/rebalance orders by `size_multiplier`
    /// (e.g. 0.5 ⇒ half notional). 0.0 ⇒ no new entries but existing
    /// positions held.
    Reduce {
        size_multiplier: f64,
        reason: String,
    },
    /// Block new entries. Existing positions untouched (funding_cycle_lock
    /// still enforces hold).
    Block { reason: String },
    /// Emergency flatten — terminate all positions at market. Used by
    /// kill_switch and by Pacifica watchdog on sustained outage.
    Flatten { reason: String },
}

impl RiskDecision {
    #[inline]
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            RiskDecision::Block { .. } | RiskDecision::Flatten { .. }
        )
    }
    #[inline]
    pub fn size_multiplier(&self) -> f64 {
        match self {
            RiskDecision::Pass => 1.0,
            RiskDecision::Reduce {
                size_multiplier, ..
            } => *size_multiplier,
            RiskDecision::Block { .. } | RiskDecision::Flatten { .. } => 0.0,
        }
    }
}
