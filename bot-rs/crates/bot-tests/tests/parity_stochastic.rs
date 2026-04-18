//! Python parity harness for `bot_strategy_v3::stochastic` + `cvar_drawdown_stop`.
//!
//! Loads fixtures from `strategy/rust_fixtures/{fit_ou,adf,cvar}.json`
//! and asserts bit-level equality within the documented `tolerance`.

use serde::Deserialize;
use std::path::PathBuf;

use bot_strategy_v3::stochastic::{adf_test, cvar_drawdown_stop, fit_drift, fit_ou};

fn fixtures_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DOL_MATH_PARITY_DIR") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("strategy")
        .join("rust_fixtures")
}

fn approx_eq(actual: f64, expected: f64, tol: f64) -> bool {
    if actual.is_nan() && expected.is_nan() {
        return true;
    }
    if !actual.is_finite() || !expected.is_finite() {
        return actual == expected;
    }
    (actual - expected).abs() <= tol.max(f64::EPSILON)
}

// ── fit_ou ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FitOuCase {
    name: String,
    input: FitOuInput,
    expected: FitOuExpected,
    tolerance: f64,
    #[allow(dead_code)]
    #[serde(default)]
    notes: Option<String>,
}
#[derive(Deserialize)]
struct FitOuInput {
    sample: Vec<f64>,
    dt: f64,
}
#[derive(Deserialize)]
struct FitOuExpected {
    mu: f64,
    theta: f64,
    sigma: f64,
    /// Missing in drift cases (theta == 0 ⇒ half_life is effectively ∞).
    #[serde(default)]
    half_life_h: Option<f64>,
    t_statistic: f64,
}

#[test]
fn parity_fit_ou() {
    let path = fixtures_dir().join("fit_ou.json");
    let contents = std::fs::read_to_string(&path).expect("read fit_ou.json");
    let cases: Vec<FitOuCase> = serde_json::from_str(&contents).expect("parse fit_ou.json");
    assert!(!cases.is_empty(), "no fit_ou cases");
    let mut failures = Vec::new();
    for case in &cases {
        let series: Vec<(i64, f64)> = case
            .input
            .sample
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as i64, v))
            .collect();
        // Python dispatches: "fit_drift_*" → fit_drift, others → fit_ou.
        let is_drift = case.name.starts_with("fit_drift_");
        let (mu, theta, sigma, half_life, t_stat) = if is_drift {
            match fit_drift(&series, case.input.dt) {
                Ok(f) => (f.mu, f.theta, f.sigma, f.half_life_h, f.t_statistic),
                Err(e) => {
                    failures.push(format!("[{}] fit_drift err: {:?}", case.name, e));
                    continue;
                }
            }
        } else {
            match fit_ou(&series, case.input.dt) {
                Ok(f) => (f.mu, f.theta, f.sigma, f.half_life_h, f.t_statistic),
                Err(e) => {
                    failures.push(format!("[{}] fit_ou err: {:?}", case.name, e));
                    continue;
                }
            }
        };
        // Bundle into a struct so the checks loop below reads uniformly.
        struct Fit {
            mu: f64,
            theta: f64,
            sigma: f64,
            half_life_h: f64,
            t_statistic: f64,
        }
        let fit = Fit {
            mu,
            theta,
            sigma,
            half_life_h: half_life,
            t_statistic: t_stat,
        };
        let mut checks: Vec<(&'static str, f64, f64)> = vec![
            ("mu", fit.mu, case.expected.mu),
            ("theta", fit.theta, case.expected.theta),
            ("sigma", fit.sigma, case.expected.sigma),
            ("t_statistic", fit.t_statistic, case.expected.t_statistic),
        ];
        if let Some(hl) = case.expected.half_life_h {
            checks.push(("half_life_h", fit.half_life_h, hl));
        }
        for (name, got, exp) in checks {
            if !approx_eq(got, exp, case.tolerance) {
                failures.push(format!(
                    "[{}] {}: got={} exp={} tol={} diff={:.3e}",
                    case.name,
                    name,
                    got,
                    exp,
                    case.tolerance,
                    (got - exp).abs()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} fit_ou parity failures:
{}",
        failures.len(),
        cases.len(),
        failures.join(
            "
"
        )
    );
    println!("parity: fit_ou.json OK ({} cases)", cases.len());
}

// ── adf_test ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AdfCase {
    name: String,
    input: AdfInput,
    expected: AdfExpected,
    tolerance: f64,
    #[allow(dead_code)]
    #[serde(default)]
    notes: Option<String>,
}
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
    let path = fixtures_dir().join("adf.json");
    let contents = std::fs::read_to_string(&path).expect("read adf.json");
    let cases: Vec<AdfCase> = serde_json::from_str(&contents).expect("parse adf.json");
    assert!(!cases.is_empty(), "no adf cases");
    let mut failures = Vec::new();
    for case in &cases {
        let r = match adf_test(&case.input.full_sample) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("[{}] adf err: {:?}", case.name, e));
                continue;
            }
        };
        if !approx_eq(r.statistic, case.expected.statistic, case.tolerance) {
            failures.push(format!(
                "[{}] statistic: got={} exp={} tol={}",
                case.name, r.statistic, case.expected.statistic, case.tolerance
            ));
        }
        if r.rejects_unit_root != case.expected.rejects_unit_root {
            failures.push(format!(
                "[{}] rejects_unit_root: got={} exp={}",
                case.name, r.rejects_unit_root, case.expected.rejects_unit_root
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} adf parity failures:
{}",
        failures.len(),
        cases.len(),
        failures.join(
            "
"
        )
    );
    println!("parity: adf.json OK ({} cases)", cases.len());
}

// ── cvar_drawdown_stop ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CvarCase {
    name: String,
    input: CvarInput,
    expected: CvarExpected,
    tolerance: f64,
    #[allow(dead_code)]
    #[serde(default)]
    notes: Option<String>,
}
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
fn parity_cvar_drawdown_stop() {
    let path = fixtures_dir().join("cvar.json");
    let contents = std::fs::read_to_string(&path).expect("read cvar.json");
    let cases: Vec<CvarCase> = serde_json::from_str(&contents).expect("parse cvar.json");
    assert!(!cases.is_empty(), "no cvar cases");
    let mut failures = Vec::new();
    for case in &cases {
        let series: Vec<(i64, f64)> = case
            .input
            .basis_history
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as i64, v))
            .collect();
        let got = cvar_drawdown_stop(&series, case.input.q, case.input.safety_multiplier);
        if !approx_eq(got, case.expected.result, case.tolerance) {
            failures.push(format!(
                "[{}] got={} exp={} tol={} diff={:.3e}",
                case.name,
                got,
                case.expected.result,
                case.tolerance,
                (got - case.expected.result).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} cvar parity failures:
{}",
        failures.len(),
        cases.len(),
        failures.join(
            "
"
        )
    );
    println!("parity: cvar.json OK ({} cases)", cases.len());
}
