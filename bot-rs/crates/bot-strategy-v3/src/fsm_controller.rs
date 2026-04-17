//! Rust port of `strategy/strategy/fsm_controller.py`.
//!
//! Aurora-Ω §22, §23 fail-safe FSM + Kelly/Neutral/Robust selector.
//!
//! # Five red-flag axes
//!
//! 4 risk layers (`entropic_ce`, `ecv`, `cvar`, `execution_chi2`) + forecast.
//! When ≥ 2 flags fire → notional × 0.4, emergency flatten 2 min, shrunken
//! IOC window, retry budget reduced.
//!
//! # Mode transitions (Aurora-Ω §22 Table 22.1)
//!
//! ```text
//!   Kelly-safe  ← 0 red flags AND funding healthy AND forecast stable
//!   Neutral     ← 1 red flag OR forecast uncertain
//!   Robust      ← ≥ 2 red flags OR forecast deterioration OR chi² spike
//! ```
//!
//! # Constants (exact match with `fsm_controller.py`)
//!
//! - `RED_FLAG_LIMIT = 2`
//! - `NOTIONAL_SCALE_ROBUST = 0.4`
//! - `EMERGENCY_FLATTEN_SECONDS = 120`
//! - `RETRY_BUDGET_ROBUST = 1`
//! - `IOC_WINDOW_ROBUST_MS = 80`
//! - `IOC_WINDOW_NEUTRAL_MS = 140`
//! - `IOC_WINDOW_KELLY_MS = 180`

use serde::{Deserialize, Serialize};

pub const RED_FLAG_LIMIT: usize = 2;
pub const NOTIONAL_SCALE_ROBUST: f64 = 0.4;
pub const EMERGENCY_FLATTEN_SECONDS: i64 = 120;
pub const RETRY_BUDGET_ROBUST: u32 = 1;
pub const IOC_WINDOW_ROBUST_MS: f64 = 80.0;
pub const IOC_WINDOW_NEUTRAL_MS: f64 = 140.0;
pub const IOC_WINDOW_KELLY_MS: f64 = 180.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[serde(rename = "kelly_safe")]
    KellySafe,
    #[serde(rename = "neutral")]
    Neutral,
    #[serde(rename = "robust")]
    Robust,
}

/// A single risk-layer report the FSM inspects. Mirrors the subset of
/// `risk_stack.RiskReport` that step() reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskReport {
    pub layer: String,
    pub red_flag: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsmDecision {
    pub mode: Mode,
    pub red_flags_fired: Vec<String>,
    pub notional_scale: f64,
    pub emergency_flatten: bool,
    pub emergency_flatten_seconds: i64,
    pub retry_budget: u32,
    pub ioc_window_ms: f64,
    pub rationale: String,
}

#[derive(Debug, Clone, Default)]
pub struct FsmState {
    pub mode: Option<Mode>,
    pub last_flatten_at: f64,
}

fn collect_flags(reports: &[RiskReport], forecast_flag: bool) -> Vec<String> {
    let mut fired: Vec<String> = reports
        .iter()
        .filter(|r| r.red_flag)
        .map(|r| r.layer.clone())
        .collect();
    if forecast_flag {
        fired.push("forecast".to_string());
    }
    fired
}

/// Advance the FSM by one tick.
///
/// `cooldown_active` — when true, the caller has an emergency_flatten timer
/// running; FSM sticks in Robust until the caller clears it.
///
/// Byte-compatible semantics with Python `fsm_controller.step`.
pub fn step(
    state: &mut FsmState,
    now_s: f64,
    reports: &[RiskReport],
    forecast_flag: bool,
    funding_healthy: bool,
    cooldown_active: bool,
) -> FsmDecision {
    let fired = collect_flags(reports, forecast_flag);
    let n_fired = fired.len();

    if n_fired >= RED_FLAG_LIMIT || cooldown_active {
        state.mode = Some(Mode::Robust);
        state.last_flatten_at = now_s;
        let emergency = n_fired >= RED_FLAG_LIMIT;
        let rationale = if emergency {
            format!("red_flags≥{RED_FLAG_LIMIT}: {:?}", fired)
        } else {
            "cooldown_active".to_string()
        };
        return FsmDecision {
            mode: Mode::Robust,
            red_flags_fired: fired,
            notional_scale: NOTIONAL_SCALE_ROBUST,
            emergency_flatten: emergency,
            emergency_flatten_seconds: EMERGENCY_FLATTEN_SECONDS,
            retry_budget: RETRY_BUDGET_ROBUST,
            ioc_window_ms: IOC_WINDOW_ROBUST_MS,
            rationale,
        };
    }

    if n_fired == 1 {
        state.mode = Some(Mode::Neutral);
        let only = fired[0].clone();
        return FsmDecision {
            mode: Mode::Neutral,
            red_flags_fired: fired,
            notional_scale: 0.75,
            emergency_flatten: false,
            emergency_flatten_seconds: 0,
            retry_budget: 2,
            ioc_window_ms: IOC_WINDOW_NEUTRAL_MS,
            rationale: format!("1 red flag: {only}"),
        };
    }

    if funding_healthy {
        state.mode = Some(Mode::KellySafe);
        return FsmDecision {
            mode: Mode::KellySafe,
            red_flags_fired: Vec::new(),
            notional_scale: 1.0,
            emergency_flatten: false,
            emergency_flatten_seconds: 0,
            retry_budget: 3,
            ioc_window_ms: IOC_WINDOW_KELLY_MS,
            rationale: "all green + funding healthy".to_string(),
        };
    }

    state.mode = Some(Mode::Neutral);
    FsmDecision {
        mode: Mode::Neutral,
        red_flags_fired: Vec::new(),
        notional_scale: 0.85,
        emergency_flatten: false,
        emergency_flatten_seconds: 0,
        retry_budget: 2,
        ioc_window_ms: IOC_WINDOW_NEUTRAL_MS,
        rationale: "clean but funding unhealthy".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §24 Banach-contraction parameter damping — `damped_update`
// ─────────────────────────────────────────────────────────────────────────────

pub const DEFAULT_MAX_STEP: f64 = 0.02;

/// Damped θ_{t+1} = (λ · E[R|θ] + β · u(θ)) / (β + λ), hard-clipped to
/// `|θ_{t+1} − θ_t| ≤ max_step`. Returns `(clipped, raw)` so the operator
/// can log the Lipschitz constant empirically.
///
/// Ref: Banach fixed-point theorem; Aurora-Ω §24.
pub fn damped_update(
    theta_t: f64,
    expected_reward: f64,
    utility: f64,
    lambda: f64,
    beta: f64,
    max_step: f64,
) -> (f64, f64) {
    let denom = beta + lambda;
    let raw = if denom > 0.0 {
        (lambda * expected_reward + beta * utility) / denom
    } else {
        theta_t
    };
    let delta = raw - theta_t;
    let clipped = if delta.abs() > max_step {
        theta_t + max_step.copysign(delta)
    } else {
        raw
    };
    (clipped, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rr(layer: &str, red: bool) -> RiskReport {
        RiskReport {
            layer: layer.to_string(),
            red_flag: red,
        }
    }

    #[test]
    fn zero_flags_healthy_is_kelly_safe() {
        let mut s = FsmState::default();
        let d = step(
            &mut s,
            0.0,
            &[rr("entropic_ce", false), rr("cvar", false)],
            false,
            true,
            false,
        );
        assert_eq!(d.mode, Mode::KellySafe);
        assert_eq!(d.notional_scale, 1.0);
        assert!(!d.emergency_flatten);
    }

    #[test]
    fn zero_flags_unhealthy_is_neutral() {
        let mut s = FsmState::default();
        let d = step(&mut s, 0.0, &[], false, false, false);
        assert_eq!(d.mode, Mode::Neutral);
        assert_eq!(d.notional_scale, 0.85);
    }

    #[test]
    fn one_flag_is_neutral_with_reduced_scale() {
        let mut s = FsmState::default();
        let d = step(&mut s, 0.0, &[rr("cvar", true)], false, true, false);
        assert_eq!(d.mode, Mode::Neutral);
        assert_eq!(d.notional_scale, 0.75);
        assert_eq!(d.red_flags_fired, vec!["cvar".to_string()]);
    }

    #[test]
    fn two_flags_trigger_robust_and_flatten() {
        let mut s = FsmState::default();
        let d = step(
            &mut s,
            100.0,
            &[rr("cvar", true), rr("ecv", true)],
            false,
            true,
            false,
        );
        assert_eq!(d.mode, Mode::Robust);
        assert!(d.emergency_flatten);
        assert_eq!(d.emergency_flatten_seconds, 120);
        assert_eq!(d.notional_scale, 0.4);
        assert_eq!(s.last_flatten_at, 100.0);
    }

    #[test]
    fn forecast_flag_counts_as_one_red() {
        let mut s = FsmState::default();
        let d = step(&mut s, 0.0, &[rr("cvar", true)], true, true, false);
        // 1 from cvar + 1 from forecast = 2 → Robust
        assert_eq!(d.mode, Mode::Robust);
        assert_eq!(d.red_flags_fired.len(), 2);
    }

    #[test]
    fn cooldown_active_sticks_in_robust() {
        let mut s = FsmState::default();
        let d = step(&mut s, 0.0, &[], false, true, true);
        assert_eq!(d.mode, Mode::Robust);
        assert!(!d.emergency_flatten); // not re-arming, just cooldown
    }

    #[test]
    fn damped_update_clips_large_moves() {
        // theta_t=0.1, reward=1.0, utility=1.0, λ=β=1, raw = 1.0 → delta=0.9.
        // max_step=0.02 → clipped θ_{t+1} = 0.12.
        let (clipped, raw) = damped_update(0.1, 1.0, 1.0, 1.0, 1.0, 0.02);
        assert_eq!(raw, 1.0);
        assert!((clipped - 0.12).abs() < 1e-12);
    }

    #[test]
    fn damped_update_passes_small_moves_unclipped() {
        // Small delta within max_step → no clipping.
        let (clipped, raw) = damped_update(0.1, 0.101, 0.1, 1.0, 1.0, 0.02);
        assert_eq!(raw, 0.1005);
        assert_eq!(clipped, raw);
    }

    #[test]
    fn damped_update_handles_zero_weights() {
        let (clipped, raw) = damped_update(0.5, 1.0, 1.0, 0.0, 0.0, 0.02);
        // Both weights 0 → no update signal, both = theta_t.
        assert_eq!(raw, 0.5);
        assert_eq!(clipped, 0.5);
    }
}
