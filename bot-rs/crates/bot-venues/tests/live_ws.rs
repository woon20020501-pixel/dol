//! Live WebSocket integration tests — connect to real mainnet endpoints.
//!
//! Marked `#[ignore = "Requires live Pacifica WebSocket at wss://ws.pacifica.fi/ws. Run: cargo test -p bot-venues --test live_ws -- --ignored. Network + 30s runtime."]` so they don't run in CI. Run manually:
//!   cargo test -p bot-venues --test live_ws -- --ignored

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Mutex};
use tokio::time;

use bot_venues::event::VenueEvent;
use bot_venues::net::CircuitBreaker;

// ── Pacifica live test ─────────────────────────────────────────

#[tokio::test]
#[ignore = "Requires live Pacifica WebSocket at wss://ws.pacifica.fi/ws. Run: cargo test -p bot-venues --test live_ws -- --ignored. Network + 30s runtime."]
async fn pacifica_ws_live_funding() {
    use bot_venues::pacifica::ws::{run_ws_loop, WsCache};

    let cache = Arc::new(Mutex::new(WsCache::default()));
    let circuit = Arc::new(Mutex::new(CircuitBreaker::default_production()));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let ws_handle = tokio::spawn(run_ws_loop(
        "wss://ws.pacifica.fi/ws".into(),
        "USDJPY".into(),
        cache,
        circuit,
        event_tx,
        shutdown_rx,
    ));

    let mut funding_count = 0;
    let deadline = time::Instant::now() + Duration::from_secs(60);

    loop {
        if time::Instant::now() > deadline {
            break;
        }
        match time::timeout(Duration::from_secs(15), event_rx.recv()).await {
            Ok(Some(VenueEvent::Connected { venue })) => {
                println!("pacifica: connected ({})", venue);
            }
            Ok(Some(VenueEvent::FundingUpdate { venue, rate })) => {
                println!(
                    "pacifica: funding #{} — symbol={} rate={} apy={:.4}%",
                    funding_count + 1,
                    rate.symbol,
                    rate.rate_per_interval,
                    rate.apy_equivalent * 100.0
                );
                assert_eq!(venue, "pacifica");
                assert_eq!(rate.symbol, "USDJPY");
                // Sanity: hourly rate should be small (< 1% per hour)
                assert!(
                    rate.rate_per_interval.abs() < 0.08,
                    "rate_per_interval too large: {}",
                    rate.rate_per_interval
                );
                assert!(rate.interval_hours == 8.0);
                funding_count += 1;
                if funding_count >= 3 {
                    break;
                }
            }
            Ok(Some(VenueEvent::OrderbookUpdate { book, .. })) => {
                if let (Some(bid), Some(ask)) = (&book.best_bid, &book.best_ask) {
                    println!(
                        "pacifica: book — bid={} ask={} spread={:.4}",
                        bid.price,
                        ask.price,
                        ask.price - bid.price
                    );
                    assert!(bid.price > 0.0);
                    assert!(ask.price > 0.0);
                    assert!(ask.price >= bid.price, "ask should >= bid");
                }
            }
            Ok(Some(VenueEvent::Heartbeat { .. })) => {}
            Ok(Some(VenueEvent::Disconnected { reason, .. })) => {
                panic!("pacifica: unexpected disconnect: {}", reason);
            }
            Ok(None) => break,
            Err(_) => {
                println!("pacifica: no message in 15s, continuing...");
            }
        }
    }

    let _ = shutdown_tx.send(true);
    let _ = time::timeout(Duration::from_secs(3), ws_handle).await;

    assert!(
        funding_count >= 3,
        "expected >=3 live funding updates, got {}",
        funding_count
    );
}

// ── Lighter live test ──────────────────────────────────────────

#[tokio::test]
#[ignore = "Requires live Pacifica WebSocket at wss://ws.pacifica.fi/ws. Run: cargo test -p bot-venues --test live_ws -- --ignored. Network + 30s runtime."]
async fn lighter_ws_live_ticker() {
    use bot_venues::lighter::rest::LighterRest;
    use bot_venues::lighter::ws::{run_ws_loop, WsCache};

    // Step 1: resolve market_id via REST
    let rest = LighterRest::new("https://mainnet.zklighter.elliot.ai/api/v1", None, None);
    let market_id = rest
        .resolve_market_id("USDJPY")
        .await
        .expect("resolve USDJPY market_id");
    println!("lighter: resolved USDJPY → market_id={}", market_id);

    // Step 2: connect WS
    let cache = Arc::new(Mutex::new(WsCache::default()));
    let circuit = Arc::new(Mutex::new(CircuitBreaker::default_production()));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let ws_handle = tokio::spawn(run_ws_loop(
        "wss://mainnet.zklighter.elliot.ai/stream".into(),
        "USDJPY".into(),
        market_id,
        cache,
        circuit,
        event_tx,
        shutdown_rx,
    ));

    let mut ticker_count = 0;
    let deadline = time::Instant::now() + Duration::from_secs(90);

    loop {
        if time::Instant::now() > deadline {
            break;
        }
        match time::timeout(Duration::from_secs(30), event_rx.recv()).await {
            Ok(Some(VenueEvent::Connected { venue })) => {
                println!("lighter: connected ({})", venue);
            }
            Ok(Some(VenueEvent::OrderbookUpdate { venue, book })) => {
                if let (Some(bid), Some(ask)) = (&book.best_bid, &book.best_ask) {
                    println!(
                        "lighter: ticker #{} — bid={} ask={} spread={:.4}",
                        ticker_count + 1,
                        bid.price,
                        ask.price,
                        ask.price - bid.price
                    );
                    assert_eq!(venue, "lighter");
                    assert!(bid.price > 0.0, "bid should be positive");
                    assert!(ask.price > 0.0, "ask should be positive");
                    assert!(ask.price >= bid.price, "ask >= bid");
                    ticker_count += 1;
                    if ticker_count >= 3 {
                        break;
                    }
                }
            }
            Ok(Some(VenueEvent::FundingUpdate { rate, .. })) => {
                println!(
                    "lighter: funding — rate={} apy={:.4}%",
                    rate.rate_per_interval,
                    rate.apy_equivalent * 100.0
                );
            }
            Ok(Some(VenueEvent::Heartbeat { .. })) => {}
            Ok(Some(VenueEvent::Disconnected { reason, .. })) => {
                panic!("lighter: unexpected disconnect: {}", reason);
            }
            Ok(None) => break,
            Err(_) => {
                println!("lighter: no message in 30s, continuing...");
            }
        }
    }

    let _ = shutdown_tx.send(true);
    let _ = time::timeout(Duration::from_secs(3), ws_handle).await;

    assert!(
        ticker_count >= 3,
        "expected >=3 live ticker updates, got {}",
        ticker_count
    );
}
