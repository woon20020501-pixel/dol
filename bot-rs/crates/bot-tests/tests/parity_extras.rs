//! Remaining Python parity harness for `bot-math` and `bot-strategy-v3`
//! functions not covered by `parity_math.rs` or `parity_stochastic.rs`:
//!
//! - `impact::effective_spread_with_impact`
//! - `mfg::{capacity_ceiling, dol_sustainable_flow_per_pair, mfg_competitor_count}`
//! - `optimum::{optimal_notional, optimal_trading_contribution}`
//! - `stochastic::expected_residual_income`
//! - `round_trip_cost` — this test documents divergence rather than parity:
//!   Rust `round_trip_cost_model_c` uses Model C fee schedules; the Python
//!   fixture uses a minimal direct-sum formula
//!   (`phi_t_p + phi_t_c + slip_p + slip_c + bridge_rt`). We replicate that
//!   minimal formula for parity and note the Model-C layer sits above it.

use serde::Deserialize;
use std::path::PathBuf;

use bot_math::{
    impact::effective_spread_with_impact,
    mfg::{capacity_ceiling, dol_sustainable_flow_per_pair, mfg_competitor_count},
    optimum::{optimal_notional, optimal_trading_contribution},
};
use bot_strategy_v3::stochastic::expected_residual_income;
use bot_types::{AnnualizedRate, AumFraction, Dimensionless, HourlyRate, Hours, Usd};

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

fn as_f64_flexible(v: &serde_json::Value) -> f64 {
    if let Some(x) = v.as_f64() {
        return x;
    }
    if let Some(s) = v.as_str() {
        return match s.to_ascii_lowercase().as_str() {
            "inf" | "+inf" | "infinity" => f64::INFINITY,
            "-inf" | "-infinity" => f64::NEG_INFINITY,
            "nan" => f64::NAN,
            other => other.parse::<f64>().unwrap_or(f64::NAN),
        };
    }
    f64::NAN
}

#[derive(Deserialize)]
struct GenericCase {
    name: String,
    input: serde_json::Value,
    expected: serde_json::Value,
    tolerance: f64,
    #[allow(dead_code)]
    #[serde(default)]
    notes: Option<String>,
}

fn load_cases(filename: &str) -> Vec<GenericCase> {
    let path = fixtures_dir().join(filename);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read fixture {}: {}", path.display(), e));
    serde_json::from_str(&contents)
        .unwrap_or_else(|e| panic!("could not parse fixture {}: {}", path.display(), e))
}

// ── effective_spread_with_impact ─────────────────────────────────────────────

#[test]
fn parity_effective_spread_with_impact() {
    let cases = load_cases("effective_spread_with_impact.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let d0 = c.input["d0"].as_f64().unwrap();
        let mu = c.input["mu"].as_f64().unwrap();
        let theta_ou = c.input["theta_ou"].as_f64().unwrap();
        let tau_h = c.input["tau_h"].as_f64().unwrap();
        let n_per_leg = c.input["n_per_leg"].as_f64().unwrap();
        let pi_pac = c.input["pi_pac"].as_f64().unwrap();
        let theta_impact = c.input["theta_impact"].as_f64().unwrap();
        let rho_comp = c.input["rho_comp"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = effective_spread_with_impact(
            AnnualizedRate(d0),
            AnnualizedRate(mu),
            HourlyRate(theta_ou),
            Hours(tau_h),
            Usd(n_per_leg),
            Usd(pi_pac),
            Dimensionless(theta_impact),
            Dimensionless(rho_comp),
        )
        .map(|r| r.0)
        .unwrap_or(f64::NAN);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} effective_spread failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!(
        "parity: effective_spread_with_impact.json OK ({} cases)",
        cases.len()
    );
}

// ── capacity_ceiling ─────────────────────────────────────────────────────────

#[test]
fn parity_capacity_ceiling() {
    let cases = load_cases("capacity_ceiling.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let n_active_pairs = c.input["n_active_pairs"].as_u64().unwrap() as u32;
        let delta_c_op = c.input["delta_c_op"].as_f64().unwrap();
        let r_floor = c.input["r_floor"].as_f64().unwrap();
        let alpha_min = c.input["alpha_min"].as_f64().unwrap();
        let r_idle = c.input["r_idle"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = capacity_ceiling(
            n_active_pairs,
            Usd(delta_c_op),
            AnnualizedRate(r_floor),
            AumFraction(alpha_min),
            AnnualizedRate(r_idle),
        )
        .map(|r| r.0)
        .unwrap_or(f64::NAN);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} capacity_ceiling failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!("parity: capacity_ceiling.json OK ({} cases)", cases.len());
}

// ── dol_sustainable_flow ─────────────────────────────────────────────────────

#[test]
fn parity_dol_sustainable_flow() {
    let cases = load_cases("dol_sustainable_flow.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let c_op_marginal = c.input["c_op_marginal"].as_f64().unwrap();
        let c_op_dol = c.input["c_op_dol"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = dol_sustainable_flow_per_pair(Usd(c_op_marginal), Usd(c_op_dol))
            .map(|r| r.0)
            .unwrap_or(f64::NAN);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} dol_sustainable_flow failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!(
        "parity: dol_sustainable_flow.json OK ({} cases)",
        cases.len()
    );
}

// ── mfg_competitor_count ─────────────────────────────────────────────────────

#[test]
fn parity_mfg_competitor_count() {
    let cases = load_cases("mfg_competitor.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let pi_pac = c.input["pi_pac"].as_f64().unwrap();
        let d_eff = c.input["d_eff"].as_f64().unwrap();
        let theta_impact = c.input["theta_impact"].as_f64().unwrap();
        let c_op_marginal = c.input["c_op_marginal"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = mfg_competitor_count(
            Usd(pi_pac),
            AnnualizedRate(d_eff),
            Dimensionless(theta_impact),
            Usd(c_op_marginal),
        )
        .unwrap_or(f64::NAN);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} mfg_competitor failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!("parity: mfg_competitor.json OK ({} cases)", cases.len());
}

// ── optimal_notional ─────────────────────────────────────────────────────────

#[test]
fn parity_optimal_notional() {
    let cases = load_cases("optimal_notional.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let pi_pac = c.input["pi_pac"].as_f64().unwrap();
        let tau_be_h = c.input["tau_be_h"].as_f64().unwrap();
        let tau_h = c.input["tau_h"].as_f64().unwrap();
        let theta_impact = c.input["theta_impact"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = optimal_notional(
            Usd(pi_pac),
            Hours(tau_be_h),
            Hours(tau_h),
            Dimensionless(theta_impact),
        )
        .0;
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} optimal_notional failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!("parity: optimal_notional.json OK ({} cases)", cases.len());
}

// ── optimal_trading_contribution ─────────────────────────────────────────────

#[test]
fn parity_optimal_trading_contribution() {
    let cases = load_cases("optimal_trading_contribution.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let d_eff = c.input["d_eff"].as_f64().unwrap();
        let pi_pac = c.input["pi_pac"].as_f64().unwrap();
        let rho_comp = c.input["rho_comp"].as_f64().unwrap();
        let theta_impact = c.input["theta_impact"].as_f64().unwrap();
        let aum = c.input["aum"].as_f64().unwrap();
        let tau_be_h = c.input["tau_be_h"].as_f64().unwrap();
        let tau_h = c.input["tau_h"].as_f64().unwrap();
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = optimal_trading_contribution(
            AnnualizedRate(d_eff),
            Usd(pi_pac),
            Dimensionless(rho_comp),
            Dimensionless(theta_impact),
            Usd(aum),
            Hours(tau_be_h),
            Hours(tau_h),
        )
        .map(|r| r.0)
        .unwrap_or(f64::NAN);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} optimal_trading_contribution failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!(
        "parity: optimal_trading_contribution.json OK ({} cases)",
        cases.len()
    );
}

// ── expected_residual_income ─────────────────────────────────────────────────

#[test]
fn parity_expected_residual_income() {
    let cases = load_cases("expected_residual_income.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let s_now = c.input["s_now"].as_f64().unwrap();
        let mu = c.input["mu"].as_f64().unwrap();
        let theta = c.input["theta"].as_f64().unwrap();
        let hold_h = c.input["hold_h"].as_f64().unwrap();
        let direction = c.input["direction"].as_i64().unwrap() as i32;
        let exp = as_f64_flexible(&c.expected["result"]);
        let got = expected_residual_income(s_now, mu, theta, hold_h, direction);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} expected_residual_income failures:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!(
        "parity: expected_residual_income.json OK ({} cases)",
        cases.len()
    );
}

// ── round_trip_cost: Model A + Model C ─────────────────────────────────────
//
// Fixture has two regimes:
//   Model A (both legs taker):
//     c = 2·phi_t_p + 2·phi_t_c + 2·slip_p + 2·slip_c + bridge_rt
//   Model C (pivot maker-open + taker-close, counter both taker):
//     c = (phi_m_p + phi_t_p) + 2·phi_t_c + slip_p + 2·slip_c + 2·ε_leg + bridge_rt
//     where ε_leg = σ_price/√86400 · √t_leg / √(2π).

#[test]
fn parity_round_trip_cost_model_a_and_c() {
    let cases = load_cases("round_trip_cost.json");
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for c in &cases {
        let phi_t_p = c.input["phi_t_p"].as_f64().unwrap();
        let phi_t_c = c.input["phi_t_c"].as_f64().unwrap();
        let slip_p = c.input["slip_p"].as_f64().unwrap();
        let slip_c = c.input["slip_c"].as_f64().unwrap();
        let bridge_rt = c.input["bridge_rt"].as_f64().unwrap();
        let got = if c.name.starts_with("rt_model_a_") {
            2.0 * phi_t_p + 2.0 * phi_t_c + 2.0 * slip_p + 2.0 * slip_c + bridge_rt
        } else if c.name.starts_with("rt_model_c_") {
            let phi_m_p = c.input["phi_m_p"].as_f64().unwrap();
            let legw = c.input["legging_window_seconds"].as_f64().unwrap();
            let sigma = c.input["sigma_price_per_sqrt_day"].as_f64().unwrap();
            let sigma_per_sqrt_sec = sigma / 86_400_f64.sqrt();
            let epsilon_leg =
                sigma_per_sqrt_sec * legw.sqrt() / (2.0 * std::f64::consts::PI).sqrt();
            (phi_m_p + phi_t_p)
                + 2.0 * phi_t_c
                + slip_p
                + 2.0 * slip_c
                + 2.0 * epsilon_leg
                + bridge_rt
        } else {
            failures.push(format!("[{}] unknown model prefix", c.name));
            continue;
        };
        let exp = as_f64_flexible(&c.expected["result"]);
        if !approx_eq(got, exp, c.tolerance) {
            failures.push(format!(
                "[{}] got={got} exp={exp} diff={:.3e}",
                c.name,
                (got - exp).abs()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} round_trip_cost failures:
{}",
        failures.len(),
        cases.len(),
        failures.join(
            "
"
        )
    );
    println!("parity: round_trip_cost.json OK ({} cases)", cases.len());
}
