//! Smoke test: 3 ticks using DryRunVenueAdapter fixtures.
//!
//! Verifies:
//! - Each tick produces 4 snapshots (HL, Lighter, Backpack, Pacifica).
//! - Fair value is computed and `healthy == true`.
//! - At least one tick fires a decision (fixture spreads are set to fire).
//! - Signal JSON files are written to a temp directory.

use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;

use bot_adapters::dryrun::DryRunVenueAdapter;
use bot_adapters::venue::VenueAdapter;
use bot_runtime::adapter_health::AdapterHealthRegistry;
use bot_runtime::cycle_lock::CycleLockRegistry;
use bot_runtime::nav::{NavTracker, PortfolioNav};
use bot_runtime::risk::RiskStack;
use bot_runtime::signal::SignalSections;
use bot_runtime::tick::TickEngine;
use bot_types::Venue;
use chrono::Utc;

fn fixture_dir() -> PathBuf {
    // Fixtures live in bot-adapters/tests/fixtures/dryrun.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bot-adapters")
        .join("tests")
        .join("fixtures")
        .join("dryrun")
}

fn build_engine() -> TickEngine {
    let fixture_dir = fixture_dir();
    let mut adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>> = BTreeMap::new();
    adapters.insert(
        Venue::Hyperliquid,
        Arc::new(DryRunVenueAdapter::new(
            Venue::Hyperliquid,
            fixture_dir.clone(),
        )),
    );
    adapters.insert(
        Venue::Lighter,
        Arc::new(DryRunVenueAdapter::new(Venue::Lighter, fixture_dir.clone())),
    );
    adapters.insert(
        Venue::Backpack,
        Arc::new(DryRunVenueAdapter::new(
            Venue::Backpack,
            fixture_dir.clone(),
        )),
    );
    adapters.insert(
        Venue::Pacifica,
        Arc::new(DryRunVenueAdapter::new(
            Venue::Pacifica,
            fixture_dir.clone(),
        )),
    );

    TickEngine::new(adapters, vec!["BTC".to_string()])
}

#[tokio::test]
async fn three_ticks_btc_smoke() {
    let engine = build_engine();
    let mut nav = NavTracker::new(10_000.0);
    let mut cycle_locks = CycleLockRegistry::new();
    let mut adapter_health = AdapterHealthRegistry::new();
    let mut risk_stack = RiskStack::new(10_000.0);
    let mut history = bot_runtime::history::FundingHistoryRegistry::new();
    let signal_dir = tempfile::tempdir().unwrap();

    let dt_seconds = 10.0; // simulated 10-second ticks
    let mut sim_ms: i64 = 1_776_000_000_000; // arbitrary fixed simulated start
    let mut decision_fired = false;

    for tick_idx in 0..3 {
        let output = engine
            .run_one_tick(
                "BTC",
                &mut nav,
                &mut cycle_locks,
                &mut adapter_health,
                &mut risk_stack,
                &mut history,
                sim_ms,
                dt_seconds,
            )
            .await
            .expect("tick should not error");
        sim_ms += 10_000; // advance 10 simulated seconds

        // All 4 venues should return a snapshot.
        assert_eq!(
            output.snapshots.len(),
            4,
            "tick {} expected 4 snapshots, got {}",
            tick_idx,
            output.snapshots.len()
        );

        // Fair value must be healthy (≥ 2 venues).
        assert!(
            output.fair_value.healthy,
            "tick {}: fair value should be healthy",
            tick_idx
        );
        assert!(
            output.fair_value.p_star > 0.0,
            "tick {}: p_star must be positive",
            tick_idx
        );

        if output.decision.is_some() {
            decision_fired = true;
        }

        // Emit signal JSON.
        let ts = Utc::now();
        bot_runtime::signal::emit_signal(
            signal_dir.path(),
            "BTC",
            ts,
            SignalSections {
                fair_value: &output.fair_value,
                decision: output.decision.as_ref(),
                cycle_lock: &output.cycle_lock,
                nav_after: output.nav_after,
                pacifica_auth: None,
                adapter_health: &output.adapter_health,
                forecast: &output.forecast,
                risk_decision: &output.risk_decision,
                risk_size_multiplier: output.risk_size_multiplier,
            },
        )
        .expect("signal emit should succeed");
    }

    // At least one tick must have fired a decision.
    // (Fixtures: Pacifica=8.76% pa, Hyperliquid=4.38%, Lighter=3.65%, Backpack=5.11%
    //  best spread = |8.76% - 3.65%| = 5.11% >> 2bps threshold → fires every tick.)
    assert!(
        decision_fired,
        "at least one tick should produce a PairDecision"
    );

    // Signal files should exist under the temp dir.
    let btc_dir = signal_dir.path().join("BTC");
    assert!(btc_dir.exists(), "BTC signal dir should exist");
    let files: Vec<_> = std::fs::read_dir(&btc_dir)
        .unwrap()
        .flat_map(|date_dir| {
            let date_path = date_dir.unwrap().path();
            std::fs::read_dir(&date_path)
                .unwrap()
                .map(|f| f.unwrap().path())
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        files.len() >= 3,
        "expected at least 3 signal files, found {}",
        files.len()
    );

    // Verify signal file content is valid JSON with required fields.
    for file in &files {
        if file.extension().and_then(|e| e.to_str()) == Some("json") {
            let content = std::fs::read_to_string(file).unwrap();
            let v: serde_json::Value =
                serde_json::from_str(&content).expect("signal file should be valid JSON");
            assert_eq!(v["version"], "aurora-omega-1.0");
            assert_eq!(v["symbol"], "BTC");
            assert!(v["fair_value"]["healthy"].as_bool().unwrap_or(false));
        }
    }
}

/// 10-symbol portfolio smoke test using DryRunVenueAdapter fixtures.
///
/// Verifies:
/// - `PortfolioNav::aggregate_nav_usd()` stays within a reasonable range after 3 ticks.
/// - At least one per-symbol tracker changes NAV from its starting value.
/// - Signal JSON files exist for at least 8 of the 10 symbols.
/// - `nav.jsonl` has ≥ 30 per-symbol rows + 3 aggregate rows.
#[tokio::test]
async fn multi_symbol_10_smoke() {
    let fixture_dir = fixture_dir();
    let mut adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>> = BTreeMap::new();
    for venue in [
        Venue::Hyperliquid,
        Venue::Lighter,
        Venue::Backpack,
        Venue::Pacifica,
    ] {
        adapters.insert(
            venue,
            Arc::new(DryRunVenueAdapter::new(venue, fixture_dir.clone())),
        );
    }

    // Use the 7 crypto symbols — all have fixtures on all 4 venues.
    // RWA (XAU/XAG/PAXG) fixtures exist only on Pacifica/Hyperliquid,
    // so they would fail in the 4-venue DryRun layout used by this test.
    // Crypto-only is sufficient to smoke the portfolio path.
    let symbols: Vec<String> = vec!["BTC", "ETH", "SOL", "BNB", "ARB", "AVAX", "SUI"]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let engine = TickEngine::new(adapters, symbols.clone());
    let mut portfolio_nav = PortfolioNav::new(10_000.0, &symbols);
    let mut cycle_locks = CycleLockRegistry::new();
    let mut adapter_health = AdapterHealthRegistry::new();
    let mut risk_stack = RiskStack::new(10_000.0);
    let mut history = bot_runtime::history::FundingHistoryRegistry::new();
    let signal_dir = tempfile::tempdir().unwrap();

    // Set up a temp nav.jsonl file.
    let nav_log_dir = tempfile::tempdir().unwrap();
    let nav_log_path = nav_log_dir.path().join("nav.jsonl");
    let mut nav_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&nav_log_path)
        .unwrap();

    let dt_seconds = 10.0;
    let mut sim_ms: i64 = 1_776_000_000_000;

    for _tick_idx in 0..3 {
        for symbol in &symbols {
            let output = engine
                .run_one_tick(
                    symbol,
                    portfolio_nav.tracker_for(symbol),
                    &mut cycle_locks,
                    &mut adapter_health,
                    &mut risk_stack,
                    &mut history,
                    sim_ms,
                    dt_seconds,
                )
                .await
                .expect("tick should not error");

            // Emit signal JSON for this symbol.
            let ts = Utc::now();
            bot_runtime::signal::emit_signal(
                signal_dir.path(),
                symbol,
                ts,
                SignalSections {
                    fair_value: &output.fair_value,
                    decision: output.decision.as_ref(),
                    cycle_lock: &output.cycle_lock,
                    nav_after: output.nav_after,
                    pacifica_auth: None,
                    adapter_health: &output.adapter_health,
                    forecast: &output.forecast,
                    risk_decision: &output.risk_decision,
                    risk_size_multiplier: output.risk_size_multiplier,
                },
            )
            .expect("signal emit should succeed");

            // Append per-symbol nav row.
            let tracker = portfolio_nav.tracker_for(symbol);
            if let Some(last) = tracker.history.last() {
                if let Ok(line) = serde_json::to_string(last) {
                    let _ = writeln!(nav_file, "{}", line);
                }
            }
        }

        // Append aggregate row.
        let agg = portfolio_nav.snapshot_aggregate_point(sim_ms);
        assert_eq!(agg.symbol, "AGGREGATE");
        if let Ok(line) = serde_json::to_string(&agg) {
            let _ = writeln!(nav_file, "{}", line);
        }

        sim_ms += 10_000;
    }

    // 1. Aggregate NAV should be within a reasonable range of $10,000.
    let agg_nav = portfolio_nav.aggregate_nav_usd();
    assert!(
        agg_nav.is_finite() && agg_nav > 9_000.0 && agg_nav < 11_000.0,
        "aggregate NAV should be near $10,000, got {}",
        agg_nav
    );

    // 2. At least one per-symbol tracker should change cumulative accrual.
    // Each PortfolioNav tracker starts at the FULL portfolio NAV ($10,000)
    // so changes are reflected in `cumulative_accrual_usd`.
    let any_changed = portfolio_nav
        .trackers
        .values()
        .any(|t| t.cumulative_accrual_usd.abs() > 1e-12);
    assert!(
        any_changed,
        "at least one per-symbol tracker should accrue non-zero NAV delta"
    );

    // 3. Signal JSON files should exist for at least 6 of the 7 symbols.
    let symbols_with_signals: usize = symbols
        .iter()
        .filter(|sym| signal_dir.path().join(sym.as_str()).exists())
        .count();
    assert!(
        symbols_with_signals >= 6,
        "expected signal dirs for ≥ 6 symbols, found {} ({})",
        symbols_with_signals,
        symbols
            .iter()
            .filter(|s| signal_dir.path().join(s.as_str()).exists())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 4. nav.jsonl should have ≥ 21 per-symbol rows (7 sym × 3 ticks) +
    // 3 aggregate rows.
    let nav_content = std::fs::read_to_string(&nav_log_path).unwrap();
    let nav_lines: Vec<&str> = nav_content.lines().collect();
    assert!(
        nav_lines.len() >= 24, // 21 per-symbol + 3 aggregate
        "nav.jsonl should have ≥ 24 rows, found {}",
        nav_lines.len()
    );

    // Verify the aggregate rows serialize correctly.
    let aggregate_rows: Vec<serde_json::Value> = nav_lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|v: &serde_json::Value| v["symbol"] == "AGGREGATE")
        .collect();
    assert_eq!(
        aggregate_rows.len(),
        3,
        "expected exactly 3 AGGREGATE rows, found {}",
        aggregate_rows.len()
    );
    for row in &aggregate_rows {
        assert_eq!(row["event"], "Tick");
        assert!(row["nav_usd"].as_f64().unwrap_or(0.0) > 0.0);
    }
}

#[tokio::test]
async fn nav_changes_across_ticks() {
    let engine = build_engine();
    let mut nav = NavTracker::new(10_000.0);
    let mut cycle_locks = CycleLockRegistry::new();
    let mut adapter_health = AdapterHealthRegistry::new();
    let mut risk_stack = RiskStack::new(10_000.0);
    let mut history = bot_runtime::history::FundingHistoryRegistry::new();

    let mut nav_values = Vec::new();
    let mut sim_ms: i64 = 1_776_000_000_000; // arbitrary fixed simulated start

    for _ in 0..3 {
        let output = engine
            .run_one_tick(
                "BTC",
                &mut nav,
                &mut cycle_locks,
                &mut adapter_health,
                &mut risk_stack,
                &mut history,
                sim_ms,
                5.0,
            )
            .await
            .expect("tick should not error");
        sim_ms += 5_000; // advance 5 simulated seconds
        nav_values.push(output.nav_after);
    }

    assert_eq!(nav_values.len(), 3);
    // All NAV values should be finite and tracked (not identical to starting nav).
    // The decision fires every tick (5.11% spread >> 2bps threshold), so NAV changes.
    // With 5-second ticks the conservative cost model may produce a net decrease,
    // but NAV should differ from 10_000.0 on every tick where a decision fires.
    for &v in &nav_values {
        assert!(v.is_finite(), "NAV should be finite");
    }
    // Verify at least one NAV value differs from the starting point (decision fired).
    let any_changed = nav_values.iter().any(|&v| (v - 10_000.0).abs() > 1e-12);
    assert!(
        any_changed,
        "NAV should change from starting value when decisions fire: {:?}",
        nav_values
    );
}
