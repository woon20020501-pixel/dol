//! Live smoke test for `PacificaReadOnlyAdapter`.
//!
//! **All tests in this file are `#[ignore]` by default** because they require
//! a real network connection to the Pacifica public API. Run them explicitly:
//!
//! ```bash
//! cargo test -p bot-adapters --ignored -- pacifica_live
//! ```
//!
//! These tests are intended to be run by hand before the demo to verify
//! Pacifica connectivity. They do NOT submit any orders.

use bot_adapters::{
    pacifica::{PacificaReadOnlyAdapter, PACIFICA_REST_URL},
    venue::VenueAdapter,
    OrderIntent, OrderKind,
};
use bot_types::{Usd, Venue};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_adapter() -> PacificaReadOnlyAdapter {
    PacificaReadOnlyAdapter::new(PACIFICA_REST_URL)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch a BTC snapshot from the live Pacifica public API.
///
/// Asserts that the core fields are populated and sane. Does NOT assert
/// exact price values because those change every second.
#[tokio::test]
#[ignore = "requires network access to Pacifica public API"]
async fn pacifica_btc_snapshot_live() {
    let adapter = make_adapter();

    let snap = adapter
        .fetch_snapshot("BTC")
        .await
        .expect("fetch_snapshot should succeed against live Pacifica");

    assert_eq!(snap.venue, Venue::Pacifica);
    assert_eq!(snap.symbol, "BTC");
    assert!(snap.mid_price > 0.0, "mid_price must be positive");
    assert!(snap.ts_ms > 0, "ts_ms must be positive");
    assert!(
        snap.funding_rate_annual.0.is_finite(),
        "funding_rate_annual must be finite"
    );
    assert!(
        snap.depth_top_usd >= 0.0,
        "depth_top_usd must be non-negative"
    );
    assert!(
        snap.depth_curve.len() >= 5,
        "depth_curve must have at least 5 points"
    );
    assert!(
        snap.next_funding_ts_ms > 0,
        "next_funding_ts_ms must be positive"
    );

    // Log values for human inspection during demo.
    eprintln!("--- Pacifica BTC live snapshot ---");
    eprintln!("  mid_price:            {:.2}", snap.mid_price);
    eprintln!(
        "  bid/ask:              {:.2} / {:.2}",
        snap.bid_price, snap.ask_price
    );
    eprintln!("  funding_rate_annual:  {:.6}", snap.funding_rate_annual.0);
    eprintln!("  funding_rate_hourly:  {:.8}", snap.funding_rate_hourly.0);
    eprintln!("  depth_top_usd:        {:.0}", snap.depth_top_usd);
    eprintln!("  volume_24h_usd:       {:.0}", snap.volume_24h_usd.0);
    eprintln!("  open_interest_usd:    {:.0}", snap.open_interest_usd.0);
    eprintln!("  ts_ms:                {}", snap.ts_ms);
}

/// Verify `list_symbols` returns a non-empty list (uses hard-coded fallback for now).
#[tokio::test]
#[ignore = "requires network access to Pacifica public API"]
async fn pacifica_list_symbols_live() {
    let adapter = make_adapter();
    let symbols = adapter
        .list_symbols()
        .await
        .expect("list_symbols should not error");
    assert!(!symbols.is_empty(), "symbol list must be non-empty");
    eprintln!("Pacifica symbol list: {symbols:?}");
}

/// Verify `fetch_position` returns None for read-only adapter.
#[tokio::test]
#[ignore = "requires network access to Pacifica public API"]
async fn pacifica_fetch_position_returns_none() {
    let adapter = make_adapter();
    let pos = adapter
        .fetch_position("BTC")
        .await
        .expect("fetch_position should not error");
    assert!(
        pos.is_none(),
        "read-only adapter must return None for position"
    );
}

/// Verify `submit_dryrun` returns a fill with dry_run=true and makes NO network order.
#[tokio::test]
#[ignore = "requires network access to Pacifica public API"]
async fn pacifica_submit_dryrun_no_live_order() {
    let adapter = make_adapter();

    // Fetch a live price first so the dryrun fill price is realistic.
    let snap = adapter.fetch_snapshot("BTC").await.unwrap();

    let order = OrderIntent {
        venue: Venue::Pacifica,
        symbol: "BTC".to_string(),
        side: 1,
        notional_usd: Usd(100.0),
        limit_price: Some(snap.mid_price),
        kind: OrderKind::TakerIoc,
        client_tag: "pacifica-live-dryrun-smoke".to_string(),
    };

    let fill = adapter
        .submit_dryrun(&order)
        .await
        .expect("submit_dryrun should succeed");

    assert!(
        fill.dry_run,
        "fill must have dry_run=true — no live order was submitted"
    );
    assert_eq!(fill.venue, Venue::Pacifica);
    assert_eq!(fill.fees_paid_usd.0, 0.0);
    assert_eq!(fill.realized_slippage_bps, 0.0);

    eprintln!(
        "Dry-run fill: price={:.2} notional={:.2}",
        fill.avg_fill_price, fill.filled_notional_usd.0
    );
}
