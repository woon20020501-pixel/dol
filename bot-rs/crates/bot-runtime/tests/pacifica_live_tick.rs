//! Live smoke test against the real Pacifica public API.
//!
//! Marked `#[ignore]` — run explicitly with:
//! ```
//! cargo test -p bot-runtime --test pacifica_live_tick -- --ignored
//! ```
//!
//! Requires network access and a live Pacifica REST endpoint.
//! Verifies the end-to-end pipeline works before hackathon day.

use std::collections::BTreeMap;
use std::sync::Arc;

use bot_adapters::pacifica::PacificaReadOnlyAdapter;
use bot_adapters::venue::VenueAdapter;
use bot_runtime::adapter_health::AdapterHealthRegistry;
use bot_runtime::cycle_lock::CycleLockRegistry;
use bot_runtime::nav::NavTracker;
use bot_runtime::risk::RiskStack;
use bot_runtime::tick::TickEngine;
use bot_types::Venue;

#[tokio::test]
#[ignore = "Live Pacifica REST round-trip for a BTC tick. Run: cargo test -p bot-runtime --test pacifica_live_tick -- --ignored. Requires network access to api.pacifica.fi; no credentials."]
async fn pacifica_live_one_tick_btc() {
    let mut adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>> = BTreeMap::new();
    adapters.insert(
        Venue::Pacifica,
        Arc::new(PacificaReadOnlyAdapter::production()),
    );

    let engine = TickEngine::new(adapters, vec!["BTC".to_string()]);
    let mut nav = NavTracker::new(10_000.0);
    let mut cycle_locks = CycleLockRegistry::new();
    let mut adapter_health = AdapterHealthRegistry::new();
    let mut risk_stack = RiskStack::new(10_000.0);
    let mut history = bot_runtime::history::FundingHistoryRegistry::new();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let output = engine
        .run_one_tick(
            "BTC",
            &mut nav,
            &mut cycle_locks,
            &mut adapter_health,
            &mut risk_stack,
            &mut history,
            now_ms,
            0.0,
        )
        .await
        .expect("live Pacifica tick should succeed");

    assert_eq!(
        output.snapshots.len(),
        1,
        "expected exactly 1 snapshot from Pacifica"
    );
    assert!(
        output.snapshots[0].mid_price > 0.0,
        "mid_price should be positive"
    );
    assert_eq!(output.snapshots[0].venue, Venue::Pacifica);

    println!(
        "Live tick: BTC mid={:.2} funding_annual={:.4}%",
        output.snapshots[0].mid_price,
        output.snapshots[0].funding_rate_annual.0 * 100.0
    );
}
