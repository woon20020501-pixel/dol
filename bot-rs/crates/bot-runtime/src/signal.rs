//! Signal JSON emitter — writes §5.2-shaped signals to disk.
//!
//! Write path: `{signal_dir}/{symbol}/{YYYYMMDD}/{ts_unix}.json`.
//! Emission is **atomic**: write to a temp file, then rename to the final path.
//!
//! Per integration-spec §5.3: signal must be written BEFORE any adapter
//! submission. Step B doesn't submit, but this ordering is preserved so Step C
//! can add submission without restructuring.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::adapter_health::SymbolHealth;
use crate::decision::PairDecision;
use crate::fair_value::FairValue;
use crate::risk::RiskDecision;
use crate::scoring::ForecastScore;
use crate::tick::CycleLockInfo;
use bot_types::Venue;

// ── Sub-structs for the §5.2 schema ──────────────────────────────────────────

/// Real cycle_lock payload populated from `CycleLockInfo`.
/// NOT stubbed — this reflects the actual enforce() result from
/// `bot_strategy_v3::funding_cycle_lock` via `cycle_lock::CycleLockRegistry`.
#[derive(Debug, Serialize)]
struct CycleLockPayload {
    locked: bool,
    cycle_index: i64,
    h_c: i8,
    #[serde(rename = "N_c")]
    n_c: f64,
    seconds_to_cycle_end: f64,
    emergency_override: bool,
    opened_new_cycle: bool,
    proposed_was_blocked: bool,
}

impl From<&CycleLockInfo> for CycleLockPayload {
    fn from(info: &CycleLockInfo) -> Self {
        Self {
            locked: info.locked,
            cycle_index: info.cycle_index,
            h_c: info.h_c,
            n_c: info.n_c,
            seconds_to_cycle_end: info.seconds_to_cycle_end,
            emergency_override: info.emergency_override,
            opened_new_cycle: info.opened_new_cycle,
            proposed_was_blocked: info.proposed_was_blocked,
        }
    }
}

/// Forecast scoring block — NOT stubbed. Populated from
/// `scoring::ForecastScore` via `ForecastScore::from`. Contains the regime
/// classification, OU fit parameters, break-even hold, Bernstein leverage
/// bound, and expected residual income — all from real math modules.
#[derive(Debug, Serialize)]
struct ForecastScoringLive {
    regime: &'static str,
    adf_statistic: Option<f64>,
    theta_hourly: Option<f64>,
    mu_annual: Option<f64>,
    drift_t_statistic: Option<f64>,
    tau_be_hours: Option<f64>,
    leverage_bound: Option<u32>,
    expected_residual_hourly: Option<f64>,
    verdict: &'static str,
}

impl From<&ForecastScore> for ForecastScoringLive {
    fn from(f: &ForecastScore) -> Self {
        use crate::scoring::{ForecastVerdict, Regime};
        Self {
            regime: match f.regime {
                Regime::Stationary => "stationary",
                Regime::Drift => "drift",
                Regime::Insufficient => "insufficient",
            },
            adf_statistic: f.adf_statistic,
            theta_hourly: f.theta_hourly,
            mu_annual: f.mu_annual,
            drift_t_statistic: f.drift_t_statistic,
            tau_be_hours: f.tau_be_hours,
            leverage_bound: f.leverage_bound,
            expected_residual_hourly: f.expected_residual_hourly,
            verdict: match f.verdict {
                ForecastVerdict::Admit => "admit",
                ForecastVerdict::Reduce => "reduce",
                ForecastVerdict::Reject => "reject",
            },
        }
    }
}

/// Runtime risk stack payload — NOT stubbed. Populated from
/// `risk::RiskDecision` emitted by `RiskStack::evaluate`.
#[derive(Debug, Serialize)]
struct RiskStackLive {
    decision: &'static str,
    size_multiplier: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl From<(&RiskDecision, f64)> for RiskStackLive {
    fn from((d, mult): (&RiskDecision, f64)) -> Self {
        let (decision_str, reason) = match d {
            RiskDecision::Pass => ("pass", None),
            RiskDecision::Reduce { reason, .. } => ("reduce", Some(reason.clone())),
            RiskDecision::Block { reason } => ("block", Some(reason.clone())),
            RiskDecision::Flatten { reason } => ("flatten", Some(reason.clone())),
        };
        Self {
            decision: decision_str,
            size_multiplier: mult,
            reason,
        }
    }
}

#[derive(Debug, Serialize)]
struct FsmStub {
    mode: &'static str,
    notional_scale: f64,
    emergency_flatten: bool,
    _stub: bool,
}

#[derive(Debug, Serialize)]
struct DiagnosticsStub {
    framework_commit: &'static str,
    bot_commit: &'static str,
    stubbed_sections: &'static [&'static str],
    /// Whether the authenticated Pacifica adapter is in use (Week 1 Task 5).
    pacifica_authenticated: bool,
    /// Builder code (public identifier, safe to log). None when auth not used.
    #[serde(skip_serializing_if = "Option::is_none")]
    builder_code: Option<String>,
    /// Oracle divergence risk annotation for RWA pairs.
    ///
    /// - `"structural"` for XAU/XAG/PAXG — Pacifica and the HL `xyz:GOLD`/
    ///   `xyz:SILVER`/PAXG legs use independent oracles, so a small basis
    ///   can open even when nothing is "wrong". Structural tail risk is
    ///   accepted and sized via venue concentration caps.
    /// - `"minimal"` for crypto pairs — both legs reference the same
    ///   oracle abstraction (BTC-PERP on both sides), so drift is
    ///   execution-noise only.
    ///
    /// The dashboard renders a warning glyph on rows flagged `structural`
    /// so readers see the honesty.
    oracle_divergence_risk: &'static str,
    /// Per-symbol adapter fetch health (telemetry rollup).
    /// Rolled up from `AdapterHealthRegistry` each tick so the dashboard
    /// can flag flaky venues without hardcoding Pacifica/HL/etc.
    book_parse_failures: SymbolHealth,
}

/// Return the oracle-divergence class for `symbol`.
fn oracle_divergence_risk_for(symbol: &str) -> &'static str {
    match symbol {
        "XAU" | "XAG" | "PAXG" => "structural",
        _ => "minimal",
    }
}

/// Sections still stubbed. `cycle_lock`, `forecast_scoring`, and `risk_stack`
/// are no longer listed — they reflect live state from their respective
/// modules. `fsm` remains a stub until the full state-machine controller
/// ports across (Week 2+ I-KILL).
const STUBBED_SECTIONS: &[&str] = &["fsm"];

#[derive(Debug, Serialize)]
struct SignalExtra<'a> {
    pair_decision: Option<&'a PairDecision>,
    nav_after: f64,
    demo_note: &'static str,
}

/// Full signal JSON payload matching integration-spec §5.2 as closely as
/// possible. Framework modules not yet ported are marked `_stub: true`.
/// `cycle_lock` is NOT stubbed — it reflects real `funding_cycle_lock`
/// enforcement state.
#[derive(Debug, Serialize)]
struct SignalPayload<'a> {
    version: &'static str,
    ts_unix: f64,
    symbol: &'a str,

    // ── Computed ──────────────────────────────────────────────────────────
    fair_value: &'a FairValue,
    /// Real cycle_lock state (from `funding_cycle_lock::enforce`).
    cycle_lock: CycleLockPayload,

    // ── Live framework modules (forecast + risk are no longer stubbed) ───
    forecast_scoring: ForecastScoringLive,
    /// Runtime 6-guard risk stack decision (CVaR, kill switch, heartbeat,
    /// Pacifica watchdog, concentration, drawdown). Live state.
    risk_stack: RiskStackLive,
    fsm: FsmStub,

    // ── Orders ────────────────────────────────────────────────────────────
    /// Always empty in Step B — no order submission.
    orders: Vec<serde_json::Value>,

    // ── Venue exposure ────────────────────────────────────────────────────
    /// Fraction of NAV allocated per venue. For the demo this is computed
    /// from the active decision: each leg = notional / nav.
    single_venue_exposure: BTreeMap<String, f64>,

    // ── Diagnostics ───────────────────────────────────────────────────────
    diagnostics: DiagnosticsStub,

    // ── Extra (non-standard) ──────────────────────────────────────────────
    extra: SignalExtra<'a>,
}

// ── Emitter ──────────────────────────────────────────────────────────────────

/// Sections the caller assembles before emitting a signal. Grouping these
/// into a single struct keeps `emit_signal` callable as new fields are
/// added (e.g., when forecast_scoring and risk_stack come online in
/// Week 2+) without cascading signature churn.
pub struct SignalSections<'a> {
    pub fair_value: &'a FairValue,
    pub decision: Option<&'a PairDecision>,
    pub cycle_lock: &'a CycleLockInfo,
    pub nav_after: f64,
    /// Builder code from the authenticated Pacifica adapter, if in use.
    /// `Some(builder_code)` marks the signal as `pacifica_authenticated: true`.
    pub pacifica_auth: Option<&'a str>,
    /// Adapter health telemetry for this symbol.
    pub adapter_health: &'a SymbolHealth,
    /// Live forecast score — OU/ADF/breakeven/Bernstein output.
    pub forecast: &'a ForecastScore,
    /// Live risk-stack decision.
    pub risk_decision: &'a RiskDecision,
    /// Size multiplier applied to the decision's notional.
    pub risk_size_multiplier: f64,
}

/// Write a signal JSON file atomically.
pub fn emit_signal(
    signal_dir: &Path,
    symbol: &str,
    ts: DateTime<Utc>,
    sections: SignalSections<'_>,
) -> anyhow::Result<PathBuf> {
    let SignalSections {
        fair_value,
        decision,
        cycle_lock,
        nav_after,
        pacifica_auth,
        adapter_health,
        forecast,
        risk_decision,
        risk_size_multiplier,
    } = sections;
    let ts_unix = ts.timestamp() as f64 + ts.timestamp_subsec_millis() as f64 / 1000.0;
    let date_str = ts.format("%Y%m%d").to_string();
    // Include milliseconds for uniqueness within the same second.
    let file_name = format!("{}{:03}.json", ts.timestamp(), ts.timestamp_subsec_millis());

    let out_dir = signal_dir.join(symbol).join(&date_str);
    std::fs::create_dir_all(&out_dir)?;

    // Build single_venue_exposure from the active decision.
    let mut single_venue_exposure: BTreeMap<String, f64> = BTreeMap::new();
    if let Some(d) = decision {
        let frac = if nav_after > 0.0 {
            d.notional_usd / nav_after
        } else {
            0.0
        };
        single_venue_exposure.insert(venue_key(d.long_venue), frac);
        single_venue_exposure.insert(venue_key(d.short_venue), frac);
    }

    let payload = SignalPayload {
        version: "aurora-omega-1.0",
        ts_unix,
        symbol,
        fair_value,
        cycle_lock: CycleLockPayload::from(cycle_lock),
        forecast_scoring: ForecastScoringLive::from(forecast),
        risk_stack: RiskStackLive::from((risk_decision, risk_size_multiplier)),
        fsm: FsmStub {
            mode: "kelly_safe",
            notional_scale: 1.0,
            emergency_flatten: false,
            _stub: true,
        },
        orders: vec![],
        single_venue_exposure,
        diagnostics: DiagnosticsStub {
            framework_commit: "demo",
            bot_commit: "demo",
            stubbed_sections: STUBBED_SECTIONS,
            pacifica_authenticated: pacifica_auth.is_some(),
            builder_code: pacifica_auth.map(|s| s.to_string()),
            oracle_divergence_risk: oracle_divergence_risk_for(symbol),
            book_parse_failures: adapter_health.clone(),
        },
        extra: SignalExtra {
            pair_decision: decision,
            nav_after,
            demo_note: "This is a Week 1 hackathon demo build. Framework modules not yet ported are marked `_stub: true`.",
        },
    };

    let json = serde_json::to_string_pretty(&payload)?;

    // Atomic write: temp → rename.
    let temp_path = out_dir.join(format!("{}.tmp", &file_name[..file_name.len() - 5]));
    let final_path = out_dir.join(&file_name);

    std::fs::write(&temp_path, &json)?;
    std::fs::rename(&temp_path, &final_path)?;

    Ok(final_path)
}

fn venue_key(v: Venue) -> String {
    match v {
        Venue::Pacifica => "pacifica".to_string(),
        Venue::Hyperliquid => "hyperliquid".to_string(),
        Venue::Lighter => "lighter".to_string(),
        Venue::Backpack => "backpack".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair_value::FairValue;
    use crate::tick::CycleLockInfo;
    use bot_types::Venue;

    fn make_fv() -> FairValue {
        FairValue {
            p_star: 100_000.0,
            total_weight: 90_000.0,
            contributing_venues: vec![Venue::Hyperliquid, Venue::Lighter],
            healthy: true,
        }
    }

    fn idle_lock() -> CycleLockInfo {
        CycleLockInfo {
            locked: false,
            cycle_index: 0,
            h_c: 0,
            n_c: 0.0,
            seconds_to_cycle_end: 3600.0,
            emergency_override: false,
            opened_new_cycle: false,
            proposed_was_blocked: false,
        }
    }

    fn idle_forecast() -> crate::scoring::ForecastScore {
        crate::scoring::ForecastScore {
            regime: crate::scoring::Regime::Insufficient,
            adf_statistic: None,
            theta_hourly: None,
            mu_annual: None,
            drift_t_statistic: None,
            tau_be_hours: None,
            leverage_bound: None,
            expected_residual_hourly: None,
            verdict: crate::scoring::ForecastVerdict::Admit,
        }
    }

    #[test]
    fn signal_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let ts = Utc::now();
        let fv = make_fv();
        let lock = idle_lock();
        let path = emit_signal(
            dir.path(),
            "BTC",
            ts,
            SignalSections {
                fair_value: &fv,
                decision: None,
                cycle_lock: &lock,
                nav_after: 10_000.0,
                pacifica_auth: None,
                adapter_health: &SymbolHealth::default(),
                forecast: &idle_forecast(),
                risk_decision: &crate::risk::RiskDecision::Pass,
                risk_size_multiplier: 1.0,
            },
        )
        .unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["version"], "aurora-omega-1.0");
        assert_eq!(v["symbol"], "BTC");
        // cycle_lock is no longer stubbed — it reflects the real state.
        assert_eq!(v["cycle_lock"]["locked"], false);
        assert_eq!(v["cycle_lock"]["h_c"], 0);
        // fsm stays stubbed until Week 2+.
        assert_eq!(v["fsm"]["_stub"], true);
        // Task 5: pacifica_authenticated must be false when pacifica_auth=None.
        assert_eq!(v["diagnostics"]["pacifica_authenticated"], false);
        assert!(v["diagnostics"]["builder_code"].is_null());
    }

    #[test]
    fn signal_contains_pair_decision() {
        use crate::decision::PairDecision;
        let dir = tempfile::tempdir().unwrap();
        let ts = Utc::now();
        let fv = make_fv();
        let lock = idle_lock();
        let decision = PairDecision {
            long_venue: Venue::Lighter,
            short_venue: Venue::Backpack,
            symbol: "BTC".to_string(),
            spread_annual: 0.03,
            cost_fraction: 0.0015,
            net_annual: 0.0285,
            notional_usd: 100.0,
            reason: "test".to_string(),
            would_have_executed: true,
        };
        let path = emit_signal(
            dir.path(),
            "BTC",
            ts,
            SignalSections {
                fair_value: &fv,
                decision: Some(&decision),
                cycle_lock: &lock,
                nav_after: 10_000.0,
                pacifica_auth: None,
                adapter_health: &SymbolHealth::default(),
                forecast: &idle_forecast(),
                risk_decision: &crate::risk::RiskDecision::Pass,
                risk_size_multiplier: 1.0,
            },
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["extra"]["pair_decision"]["symbol"], "BTC");
        assert_eq!(v["extra"]["nav_after"], 10_000.0);
    }
    #[test]
    fn signal_diagnostics_pacifica_authenticated_when_auth_provided() {
        let dir = tempfile::tempdir().unwrap();
        let ts = Utc::now();
        let fv = make_fv();
        let lock = idle_lock();
        let path = emit_signal(
            dir.path(),
            "BTC",
            ts,
            SignalSections {
                fair_value: &fv,
                decision: None,
                cycle_lock: &lock,
                nav_after: 10_000.0,
                pacifica_auth: Some("BLDR42"),
                adapter_health: &SymbolHealth::default(),
                forecast: &idle_forecast(),
                risk_decision: &crate::risk::RiskDecision::Pass,
                risk_size_multiplier: 1.0,
            },
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["diagnostics"]["pacifica_authenticated"], true);
        assert_eq!(v["diagnostics"]["builder_code"], "BLDR42");
    }
}
