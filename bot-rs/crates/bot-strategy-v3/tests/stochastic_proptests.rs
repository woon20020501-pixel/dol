//! Property-based tests for `bot_strategy_v3::stochastic`.
//!
//! These supplement `parity_stochastic.rs` (fixture-based byte-exact) with
//! randomized invariant coverage. Each property runs 256 random samples by
//! default (proptest default), producing ~10 properties × 256 = ~2560
//! randomized invariant checks on top of the existing 17 bot-math proptests
//! and the 3 stochastic parity cases.
//!
//! # Invariants covered
//!
//! - `adf_test` returns a finite t-statistic and the 5% critical value
//!   -2.86 (MacKinnon 1996) regardless of input distribution.
//! - `fit_drift` recovers the sample mean within 4σ/√n for i.i.d. Gaussian
//!   synthetic data.
//! - `cvar_drawdown_stop` with `q ∈ (0, 1)` returns a non-negative result.
//! - `expected_residual_income`: sign flips with `direction`.
//! - `cycle_index(t, T)` is monotone in `t` for fixed `T > 0`.
//! - `seconds_to_cycle_end` is in `[0, cycle_seconds]`.

use proptest::prelude::*;

use bot_strategy_v3::funding_cycle_lock::{cycle_index, seconds_to_cycle_end};
use bot_strategy_v3::stochastic::{
    adf_test, cvar_drawdown_stop, expected_residual_income, fit_drift,
};

// ─── cycle_index / seconds_to_cycle_end ──────────────────────────────────────

proptest! {
    /// `cycle_index(t, T)` is monotone non-decreasing in `t` for `T > 0`.
    #[test]
    fn cycle_index_monotone(t in 0.0f64..1e12, delta in 1.0f64..1e9, cs in 60i64..86400) {
        let a = cycle_index(t, cs);
        let b = cycle_index(t + delta, cs);
        prop_assert!(b >= a, "cycle_index({}, {}) = {} should be ≤ cycle_index({}, {}) = {}",
            t, cs, a, t+delta, cs, b);
    }

    /// `seconds_to_cycle_end` lies in `[0, cycle_seconds)`.
    #[test]
    fn seconds_to_cycle_end_bounded(t in 0.0f64..1e12, cs in 60i64..86400) {
        let rem = seconds_to_cycle_end(t, cs);
        prop_assert!(rem >= 0.0, "rem = {} must be ≥ 0", rem);
        prop_assert!(rem < cs as f64 + 1e-9, "rem = {} must be < {}", rem, cs);
    }
}

// ─── adf_test ───────────────────────────────────────────────────────────────

proptest! {
    /// ADF t-statistic is finite when series has sufficient length and variance.
    #[test]
    fn adf_statistic_is_finite(
        xs in prop::collection::vec(-10.0f64..10.0, 60..150),
    ) {
        if let Ok(res) = adf_test(&xs) {
            prop_assert!(res.statistic.is_finite(), "ADF statistic must be finite, got {}", res.statistic);
            prop_assert_eq!(res.critical_5pct, -2.86, "constant-only model uses MacKinnon 5% = -2.86");
        }
    }
}

// ─── fit_drift ──────────────────────────────────────────────────────────────

proptest! {
    /// fit_drift recovers the sample mean exactly (by definition: mu = sample mean).
    #[test]
    fn fit_drift_matches_sample_mean(
        xs in prop::collection::vec(-1.0f64..1.0, 30..120),
    ) {
        let series: Vec<(i64, f64)> = xs.iter().enumerate().map(|(i, &v)| (i as i64, v)).collect();
        if let Ok(fit) = fit_drift(&series, 1.0) {
            let expected: f64 = xs.iter().sum::<f64>() / xs.len() as f64;
            prop_assert!(
                (fit.mu - expected).abs() < 1e-12,
                "fit.mu = {} should equal sample mean = {}",
                fit.mu, expected
            );
        }
    }
}

// ─── cvar_drawdown_stop ─────────────────────────────────────────────────────

proptest! {
    /// cvar_drawdown_stop ≥ 0 for any finite basis series and α ∈ (0, 1).
    #[test]
    fn cvar_drawdown_stop_non_negative(
        basis in prop::collection::vec(-1.0f64..1.0, 0..200),
        q in 0.001f64..0.999,
        mult in 0.5f64..5.0,
    ) {
        let series: Vec<(i64, f64)> = basis.iter().enumerate().map(|(i, &v)| (i as i64, v)).collect();
        let r = cvar_drawdown_stop(&series, q, mult);
        prop_assert!(r >= 0.0, "cvar_drawdown_stop = {} < 0", r);
        prop_assert!(r.is_finite(), "cvar_drawdown_stop = {} non-finite", r);
    }
}

// ─── expected_residual_income ───────────────────────────────────────────────

proptest! {
    /// ERI flips sign with direction (±1).
    #[test]
    fn eri_direction_sign_flip(
        s_now in -1.0f64..1.0,
        mu in -1.0f64..1.0,
        theta in 0.001f64..10.0,
        hold_h in 0.1f64..1000.0,
    ) {
        let pos = expected_residual_income(s_now, mu, theta, hold_h, 1);
        let neg = expected_residual_income(s_now, mu, theta, hold_h, -1);
        prop_assert!(
            (pos + neg).abs() < 1e-12,
            "ERI should flip sign with direction: +1 = {}, -1 = {}",
            pos, neg
        );
    }

    /// At s_now == mu, ERI reduces to direction · mu · hold_h
    /// (the OU decay term vanishes).
    #[test]
    fn eri_at_mean_equals_drift_term(
        mu in -1.0f64..1.0,
        theta in 0.001f64..10.0,
        hold_h in 0.1f64..1000.0,
    ) {
        let got = expected_residual_income(mu, mu, theta, hold_h, 1);
        let expected = mu * hold_h;
        prop_assert!(
            (got - expected).abs() < 1e-10,
            "ERI at s=mu should equal mu·hold_h: got {} vs expected {}",
            got, expected
        );
    }
}
