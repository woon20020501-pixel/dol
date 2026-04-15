//! Phase 2a parity tests — bot-strategy-v3 stochastic functions.
//!
//! One `#[test]` per fixture section:
//!   - `fit_ou`                  → `rust_fixtures/fit_ou.json`
//!   - `adf`                     → `rust_fixtures/adf.json`
//!   - `cvar`                    → `rust_fixtures/cvar.json`
//!   - `expected_residual_income`→ `rust_fixtures/expected_residual_income.json`
//!
//! `fit_drift` is covered internally in `bot-strategy-v3` (3+ unit tests).
//! The drift case in `fit_ou.json` is also tested here under `fit_ou`.

use bot_strategy_v3::stochastic::{
    adf_test, cvar_drawdown_stop, expected_residual_income, fit_drift, fit_ou as fit_ou_fn,
};
use bot_tests::{assert_close, load_fixtures, Fixture};
use serde::Deserialize;
use serde_json::Value;

// ===========================================================================
// fit_ou  (rust_fixtures/fit_ou.json)
// ===========================================================================

#[derive(Deserialize)]
struct FitOuInput {
    sample: Vec<f64>,
    dt: f64,
}

/// Expected fields for OU cases: mu, theta, sigma, half_life_h, t_statistic.
/// Drift cases have theta=0 and no half_life_h. Use `Option` for the optional ones.
#[derive(Deserialize)]
struct FitOuExpected {
    mu: f64,
    theta: f64,
    sigma: f64,
    #[serde(default)]
    half_life_h: Option<f64>,
    t_statistic: f64,
}

// Manually deserialize to handle missing `half_life_h`
fn parse_fit_ou_expected(v: &Value) -> FitOuExpected {
    FitOuExpected {
        mu: v["mu"].as_f64().unwrap(),
        theta: v["theta"].as_f64().unwrap(),
        sigma: v["sigma"].as_f64().unwrap(),
        half_life_h: v.get("half_life_h").and_then(|x| x.as_f64()),
        t_statistic: v["t_statistic"].as_f64().unwrap(),
    }
}

#[test]
fn parity_fit_ou() {
    // Use raw Value deserialization to handle optional half_life_h
    #[derive(Deserialize)]
    struct RawFixture {
        name: String,
        input: FitOuInput,
        expected: Value,
        tolerance: f64,
        #[serde(default)]
        #[allow(dead_code)]
        notes: String,
    }
    let fixtures: Vec<RawFixture> = {
        let path = bot_tests::fixtures_dir().join("fit_ou.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read fit_ou.json: {}", e));
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("Cannot parse fit_ou.json: {}", e))
    };

    for f in &fixtures {
        let series: Vec<(i64, f64)> = f
            .input
            .sample
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as i64, v))
            .collect();
        let expected = parse_fit_ou_expected(&f.expected);
        let tol = f.tolerance;

        // Drift case: theta == 0.0 in expected → call fit_drift
        if expected.theta == 0.0 {
            let result = fit_drift(&series, f.input.dt)
                .unwrap_or_else(|e| panic!("fit_drift failed for case '{}': {:?}", f.name, e));
            assert_close(result.mu, expected.mu, tol, &f.name);
            assert_close(result.theta, expected.theta, tol, &f.name);
            assert_close(result.sigma, expected.sigma, tol, &f.name);
            assert_close(result.t_statistic, expected.t_statistic, tol, &f.name);
        } else {
            // OU case
            let result = fit_ou_fn(&series, f.input.dt)
                .unwrap_or_else(|e| panic!("fit_ou failed for case '{}': {:?}", f.name, e));
            assert_close(result.mu, expected.mu, tol, &f.name);
            assert_close(result.theta, expected.theta, tol, &f.name);
            assert_close(result.sigma, expected.sigma, tol, &f.name);
            assert_close(result.t_statistic, expected.t_statistic, tol, &f.name);
            if let Some(hl) = expected.half_life_h {
                assert_close(
                    result.half_life_h,
                    hl,
                    tol,
                    &format!("{}/half_life_h", f.name),
                );
            }
        }
    }
}

// ===========================================================================
// adf  (rust_fixtures/adf.json)
// ===========================================================================

#[derive(Deserialize)]
struct AdfInput {
    full_sample: Vec<f64>,
}

#[derive(Deserialize)]
struct AdfExpected {
    statistic: f64,
    rejects_unit_root: bool,
}

#[test]
fn parity_adf() {
    let fixtures: Vec<Fixture<AdfInput, AdfExpected>> = load_fixtures("adf");
    for f in &fixtures {
        let result = adf_test(&f.input.full_sample)
            .unwrap_or_else(|e| panic!("adf_test failed for case '{}': {:?}", f.name, e));
        assert_close(result.statistic, f.expected.statistic, f.tolerance, &f.name);
        assert_eq!(
            result.rejects_unit_root, f.expected.rejects_unit_root,
            "case '{}': rejects_unit_root mismatch: got {}, expected {}",
            f.name, result.rejects_unit_root, f.expected.rejects_unit_root
        );
    }
}

// ===========================================================================
// cvar  (rust_fixtures/cvar.json)
// ===========================================================================

#[derive(Deserialize)]
struct CvarInput {
    basis_history: Vec<f64>,
    q: f64,
    safety_multiplier: f64,
}

#[derive(Deserialize)]
struct CvarExpected {
    result: f64,
}

#[test]
fn parity_cvar() {
    let fixtures: Vec<Fixture<CvarInput, CvarExpected>> = load_fixtures("cvar");
    for f in &fixtures {
        // Convert plain Vec<f64> to Vec<(i64, f64)> with dummy timestamps
        let series: Vec<(i64, f64)> = f
            .input
            .basis_history
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as i64, v))
            .collect();
        let result = cvar_drawdown_stop(&series, f.input.q, f.input.safety_multiplier);
        assert_close(result, f.expected.result, f.tolerance, &f.name);
    }
}

// ===========================================================================
// expected_residual_income  (rust_fixtures/expected_residual_income.json)
// ===========================================================================

#[derive(Deserialize)]
struct EriInput {
    s_now: f64,
    mu: f64,
    theta: f64,
    hold_h: f64,
    direction: i32,
}

#[derive(Deserialize)]
struct EriExpected {
    result: f64,
}

#[test]
fn parity_eri() {
    let fixtures: Vec<Fixture<EriInput, EriExpected>> = load_fixtures("expected_residual_income");
    for f in &fixtures {
        let result = expected_residual_income(
            f.input.s_now,
            f.input.mu,
            f.input.theta,
            f.input.hold_h,
            f.input.direction,
        );
        assert_close(result, f.expected.result, f.tolerance, &f.name);
    }
}
