//! Parity test: the FSM truth table in `docs/FSM_DIAGRAM.md` exactly matches
//! `bot_strategy_v3::fsm_controller::step`.
//!
//! Any edit to either side without updating the other will fail this test.

use bot_strategy_v3::fsm_controller::{
    step, FsmState, Mode, RiskReport, IOC_WINDOW_KELLY_MS, IOC_WINDOW_NEUTRAL_MS,
    IOC_WINDOW_ROBUST_MS, NOTIONAL_SCALE_ROBUST, RETRY_BUDGET_ROBUST,
};

fn reports(n: usize) -> Vec<RiskReport> {
    let names = ["entropic_ce", "ecv", "cvar", "execution_chi2"];
    (0..n.min(4))
        .map(|i| RiskReport {
            layer: names[i].to_string(),
            red_flag: true,
        })
        .collect()
}

/// Each tuple is `(n_red_flags, funding_healthy, cooldown_active, expected_mode,
/// expected_notional_scale, expected_emergency_flatten)`.
fn truth_table() -> Vec<(usize, bool, bool, Mode, f64, bool)> {
    vec![
        (0, true, false, Mode::KellySafe, 1.0, false),
        (0, false, false, Mode::Neutral, 0.85, false),
        (1, true, false, Mode::Neutral, 0.75, false),
        (1, false, false, Mode::Neutral, 0.75, false),
        (2, true, false, Mode::Robust, NOTIONAL_SCALE_ROBUST, true),
        (2, false, false, Mode::Robust, NOTIONAL_SCALE_ROBUST, true),
        (3, true, false, Mode::Robust, NOTIONAL_SCALE_ROBUST, true),
        (0, true, true, Mode::Robust, NOTIONAL_SCALE_ROBUST, false),
        (1, true, true, Mode::Robust, NOTIONAL_SCALE_ROBUST, false),
    ]
}

#[test]
fn fsm_matches_diagram_truth_table() {
    let mut failures = Vec::new();
    for (n, funding_ok, cooldown, exp_mode, exp_scale, exp_flatten) in truth_table() {
        let mut state = FsmState::default();
        let rs = reports(n);
        let d = step(&mut state, 0.0, &rs, false, funding_ok, cooldown);
        if d.mode != exp_mode {
            failures.push(format!(
                "({n}, healthy={funding_ok}, cooldown={cooldown}): mode = {:?}, expected {:?}",
                d.mode, exp_mode
            ));
        }
        if (d.notional_scale - exp_scale).abs() > 1e-12 {
            failures.push(format!(
                "({n}, healthy={funding_ok}, cooldown={cooldown}): notional_scale = {}, expected {}",
                d.notional_scale, exp_scale
            ));
        }
        if d.emergency_flatten != exp_flatten {
            failures.push(format!(
                "({n}, healthy={funding_ok}, cooldown={cooldown}): emergency_flatten = {}, expected {}",
                d.emergency_flatten, exp_flatten
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "FSM diagram ↔ code mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn fsm_ioc_windows_match_diagram() {
    // Kelly → 180ms, Neutral → 140ms, Robust → 80ms
    let mut state = FsmState::default();
    let d_kelly = step(&mut state, 0.0, &[], false, true, false);
    assert_eq!(d_kelly.ioc_window_ms, IOC_WINDOW_KELLY_MS);

    let mut state = FsmState::default();
    let d_neutral = step(&mut state, 0.0, &reports(1), false, true, false);
    assert_eq!(d_neutral.ioc_window_ms, IOC_WINDOW_NEUTRAL_MS);

    let mut state = FsmState::default();
    let d_robust = step(&mut state, 0.0, &reports(2), false, true, false);
    assert_eq!(d_robust.ioc_window_ms, IOC_WINDOW_ROBUST_MS);
}

#[test]
fn fsm_retry_budgets_match_diagram() {
    let mut state = FsmState::default();
    let d_kelly = step(&mut state, 0.0, &[], false, true, false);
    assert_eq!(d_kelly.retry_budget, 3);

    let mut state = FsmState::default();
    let d_neutral = step(&mut state, 0.0, &reports(1), false, true, false);
    assert_eq!(d_neutral.retry_budget, 2);

    let mut state = FsmState::default();
    let d_robust = step(&mut state, 0.0, &reports(2), false, true, false);
    assert_eq!(d_robust.retry_budget, RETRY_BUDGET_ROBUST);
}

/// Invariant 1: `Robust ⇔ (n_flags ≥ 2) ∨ cooldown_active`.
/// Enumerate the small state space (n ∈ 0..=4, booleans) and assert.
#[test]
fn fsm_robust_iff_two_flags_or_cooldown() {
    for n in 0..=4usize {
        for &healthy in &[true, false] {
            for &cooldown in &[true, false] {
                let mut state = FsmState::default();
                let d = step(&mut state, 0.0, &reports(n), false, healthy, cooldown);
                let expected_robust = n >= 2 || cooldown;
                assert_eq!(
                    d.mode == Mode::Robust,
                    expected_robust,
                    "(n={n}, healthy={healthy}, cooldown={cooldown}): mode = {:?}",
                    d.mode,
                );
            }
        }
    }
}

/// Invariant 4: `emergency_flatten ⇔ n_flags ≥ 2` (cooldown alone does NOT re-arm).
#[test]
fn fsm_emergency_flatten_iff_two_flags() {
    for n in 0..=4usize {
        for &healthy in &[true, false] {
            for &cooldown in &[true, false] {
                let mut state = FsmState::default();
                let d = step(&mut state, 0.0, &reports(n), false, healthy, cooldown);
                assert_eq!(
                    d.emergency_flatten,
                    n >= 2,
                    "(n={n}, healthy={healthy}, cooldown={cooldown}): flatten = {}",
                    d.emergency_flatten,
                );
            }
        }
    }
}
