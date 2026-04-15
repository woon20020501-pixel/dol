//! Smoke tests for `DryRunVenueAdapter`.
//!
//! Exercises all three non-Pacifica fixture venues (Hyperliquid, Lighter, Backpack)
//! without any network access. Run with `cargo test -p bot-adapters`.

use std::path::PathBuf;
use std::sync::Arc;

use bot_adapters::{dryrun::DryRunVenueAdapter, venue::VenueAdapter, OrderIntent, OrderKind};
use bot_types::{Usd, Venue};

/// Path to the fixture directory relative to the crate root.
fn fixture_dir() -> PathBuf {
    // When running `cargo test`, the working directory is the crate root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("dryrun")
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build adapter and run standard assertions
// ─────────────────────────────────────────────────────────────────────────────

async fn smoke_one_venue(venue: Venue) {
    let adapter = DryRunVenueAdapter::new(venue, fixture_dir());

    // 1. fetch_snapshot
    let snap = adapter
        .fetch_snapshot("BTC")
        .await
        .expect("fetch_snapshot should succeed for fixture venue");

    assert_eq!(
        snap.venue, venue,
        "snapshot venue should match adapter venue"
    );
    assert_eq!(snap.symbol, "BTC");
    assert!(snap.mid_price > 0.0, "mid_price must be positive");
    assert!(snap.bid_price > 0.0, "bid_price must be positive");
    assert!(snap.ask_price > 0.0, "ask_price must be positive");
    assert!(snap.ask_price >= snap.bid_price, "ask >= bid");
    assert!(snap.ts_ms > 0, "ts_ms must be positive");
    assert!(snap.tick_size > 0.0, "tick_size must be positive");
    assert!(snap.depth_top_usd > 0.0, "depth_top_usd must be positive");
    assert!(
        snap.depth_curve.len() >= 5,
        "depth_curve must have at least 5 points"
    );
    assert!(
        snap.funding_rate_annual.0.is_finite(),
        "funding_rate_annual must be finite"
    );
    assert!(
        snap.funding_rate_hourly.0.is_finite(),
        "funding_rate_hourly must be finite"
    );
    assert!(
        snap.funding_interval_seconds > 0,
        "funding_interval_seconds must be positive"
    );
    assert!(
        snap.next_funding_ts_ms > 0,
        "next_funding_ts_ms must be positive"
    );

    // 2. fetch_position — fixture adapter always returns None
    let pos = adapter
        .fetch_position("BTC")
        .await
        .expect("fetch_position should not error");
    assert!(pos.is_none(), "dryrun adapter has no positions");

    // 3. submit_dryrun — must return a fill with dry_run=true
    let order = OrderIntent {
        venue,
        symbol: "BTC".to_string(),
        side: 1,
        notional_usd: Usd(5000.0),
        limit_price: Some(snap.mid_price),
        kind: OrderKind::TakerIoc,
        client_tag: format!("smoke-test-{venue:?}-btc"),
    };

    let fill = adapter
        .submit_dryrun(&order)
        .await
        .expect("submit_dryrun should succeed");

    assert!(fill.dry_run, "fill must have dry_run=true");
    assert_eq!(fill.venue, venue);
    assert_eq!(fill.symbol, "BTC");
    assert_eq!(fill.side, 1);
    assert!((fill.filled_notional_usd.0 - 5000.0).abs() < 1e-9);
    assert!(fill.avg_fill_price > 0.0);
    assert_eq!(fill.fees_paid_usd.0, 0.0);
    assert_eq!(fill.realized_slippage_bps, 0.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — one per fixture venue
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn hyperliquid_btc_smoke() {
    smoke_one_venue(Venue::Hyperliquid).await;
}

#[tokio::test]
async fn lighter_btc_smoke() {
    smoke_one_venue(Venue::Lighter).await;
}

#[tokio::test]
async fn backpack_btc_smoke() {
    smoke_one_venue(Venue::Backpack).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// list_symbols — fixture dir scan
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_symbols_returns_btc() {
    let adapter = DryRunVenueAdapter::new(Venue::Hyperliquid, fixture_dir());
    let symbols = adapter
        .list_symbols()
        .await
        .expect("list_symbols should succeed");
    assert!(
        symbols.contains(&"BTC".to_string()),
        "BTC fixture must appear in symbol list; got: {symbols:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Trait object safety — Arc<dyn VenueAdapter> must compile
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn trait_object_safe() {
    let adapter = DryRunVenueAdapter::new(Venue::Hyperliquid, fixture_dir());
    // Constructing an Arc<dyn VenueAdapter> proves object safety at compile time.
    let _dyn_adapter: Arc<dyn VenueAdapter> = Arc::new(adapter);
}

// ─────────────────────────────────────────────────────────────────────────────
// Cyclic replay — second call returns same fixture (single-snapshot files cycle)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cyclic_replay_single_snapshot() {
    let adapter = DryRunVenueAdapter::new(Venue::Lighter, fixture_dir());

    let snap1 = adapter.fetch_snapshot("BTC").await.unwrap();
    let snap2 = adapter.fetch_snapshot("BTC").await.unwrap();

    // Single-snapshot files replay the same snapshot on every call.
    assert!(
        (snap1.mid_price - snap2.mid_price).abs() < 1e-9,
        "single-snapshot file should always return the same mid_price"
    );
}
