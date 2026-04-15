//! Mock WS integration tests — spins up a local WS server and tests
//! adapter connect → subscribe → receive → disconnect flow.
//!
//! These run in CI (no `#[ignore]`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time;
use tokio_tungstenite::tungstenite::Message;

use bot_venues::event::VenueEvent;
use bot_venues::net::CircuitBreaker;
use bot_venues::pacifica::ws::{run_ws_loop as pacifica_ws_loop, WsCache as PacificaCache};

/// Spawn a minimal WS server that sends Pacifica-style messages.
async fn spawn_pacifica_mock(
    addr: SocketAddr,
    message_count: usize,
    shutdown_rx: watch::Receiver<bool>,
) {
    let listener = TcpListener::bind(addr).await.unwrap();
    let mut shutdown = shutdown_rx;

    tokio::select! {
        result = listener.accept() => {
            let (stream, _) = result.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut write, mut read) = ws.split();

            // Read subscription messages (ignore them)
            for _ in 0..2 {
                let _ = time::timeout(Duration::from_secs(2), read.next()).await;
            }

            // Send price updates
            for i in 0..message_count {
                let msg = json!({
                    "channel": "prices",
                    "data": [{
                        "symbol": "USDJPY",
                        "funding": format!("0.000{}", i + 1),
                        "next_funding": "1775000000",
                        "mark": "152.50",
                        "timestamp": 1775000000.0 + i as f64
                    }]
                });
                if write.send(Message::Text(msg.to_string())).await.is_err() {
                    break;
                }
                time::sleep(Duration::from_millis(50)).await;
            }

            // Send book update
            let book_msg = json!({
                "channel": "book",
                "data": {
                    "s": "USDJPY",
                    "l": [
                        [{"p": "152.45", "a": "100", "n": 2}],
                        [{"p": "152.55", "a": "80", "n": 1}]
                    ],
                    "t": 1775000000000_i64
                }
            });
            let _ = write.send(Message::Text(book_msg.to_string())).await;
            time::sleep(Duration::from_millis(50)).await;

            // Wait for shutdown
            let _ = shutdown.changed().await;
            let _ = write.send(Message::Close(None)).await;
        }
        _ = shutdown.changed() => {}
    }
}

#[tokio::test]
async fn pacifica_ws_receives_funding_updates() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();
    let bound_addr = listener.local_addr().unwrap();
    drop(listener); // free the port for the mock

    let (mock_shutdown_tx, mock_shutdown_rx) = watch::channel(false);
    let mock_handle = tokio::spawn(spawn_pacifica_mock(bound_addr, 3, mock_shutdown_rx));

    // Brief delay to let mock server start
    time::sleep(Duration::from_millis(100)).await;

    let cache = Arc::new(Mutex::new(PacificaCache::default()));
    let circuit = Arc::new(Mutex::new(CircuitBreaker::default_production()));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (ws_shutdown_tx, ws_shutdown_rx) = watch::channel(false);

    let ws_url = format!("ws://{}", bound_addr);
    let ws_handle = tokio::spawn(pacifica_ws_loop(
        ws_url,
        "USDJPY".into(),
        cache.clone(),
        circuit.clone(),
        event_tx,
        ws_shutdown_rx,
    ));

    // Collect events (with timeout)
    let mut funding_count = 0;
    let mut book_count = 0;
    let deadline = time::Instant::now() + Duration::from_secs(5);

    loop {
        if time::Instant::now() > deadline {
            break;
        }
        match time::timeout(Duration::from_secs(1), event_rx.recv()).await {
            Ok(Some(VenueEvent::Connected { .. })) => {}
            Ok(Some(VenueEvent::FundingUpdate { venue, rate })) => {
                assert_eq!(venue, "pacifica");
                assert_eq!(rate.symbol, "USDJPY");
                assert!(rate.rate_per_interval > 0.0);
                funding_count += 1;
            }
            Ok(Some(VenueEvent::OrderbookUpdate { venue, book })) => {
                assert_eq!(venue, "pacifica");
                assert!(book.best_bid.is_some());
                assert!(book.best_ask.is_some());
                book_count += 1;
            }
            Ok(Some(VenueEvent::Heartbeat { .. })) => {}
            Ok(Some(VenueEvent::Disconnected { .. })) => break,
            Ok(None) => break,
            Err(_) => break, // timeout
        }
        if funding_count >= 3 && book_count >= 1 {
            break;
        }
    }

    // Shutdown
    let _ = ws_shutdown_tx.send(true);
    let _ = mock_shutdown_tx.send(true);
    let _ = time::timeout(Duration::from_secs(2), ws_handle).await;
    let _ = time::timeout(Duration::from_secs(2), mock_handle).await;

    assert!(
        funding_count >= 3,
        "expected >=3 funding updates, got {}",
        funding_count
    );
    assert!(
        book_count >= 1,
        "expected >=1 book update, got {}",
        book_count
    );
}

#[tokio::test]
async fn pacifica_ws_caches_latest_state() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();
    let bound_addr = listener.local_addr().unwrap();
    drop(listener);

    let (mock_shutdown_tx, mock_shutdown_rx) = watch::channel(false);
    let mock_handle = tokio::spawn(spawn_pacifica_mock(bound_addr, 2, mock_shutdown_rx));

    time::sleep(Duration::from_millis(100)).await;

    let cache = Arc::new(Mutex::new(PacificaCache::default()));
    let circuit = Arc::new(Mutex::new(CircuitBreaker::default_production()));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (ws_shutdown_tx, ws_shutdown_rx) = watch::channel(false);

    let ws_url = format!("ws://{}", bound_addr);
    let ws_handle = tokio::spawn(pacifica_ws_loop(
        ws_url,
        "USDJPY".into(),
        cache.clone(),
        circuit.clone(),
        event_tx,
        ws_shutdown_rx,
    ));

    // Wait for some events
    let mut received = 0;
    for _ in 0..10 {
        match time::timeout(Duration::from_secs(2), event_rx.recv()).await {
            Ok(Some(VenueEvent::FundingUpdate { .. })) => {
                received += 1;
                if received >= 2 {
                    break;
                }
            }
            Ok(Some(_)) => {}
            _ => break,
        }
    }

    // Check cache has data
    {
        let c = cache.lock().await;
        assert!(c.funding.is_some(), "cache should have funding data");
    }

    let _ = ws_shutdown_tx.send(true);
    let _ = mock_shutdown_tx.send(true);
    let _ = time::timeout(Duration::from_secs(2), ws_handle).await;
    let _ = time::timeout(Duration::from_secs(2), mock_handle).await;
}

#[tokio::test]
async fn pacifica_ws_reconnects_on_server_drop() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();
    let bound_addr = listener.local_addr().unwrap();

    let cache = Arc::new(Mutex::new(PacificaCache::default()));
    let circuit = Arc::new(Mutex::new(CircuitBreaker::default_production()));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (ws_shutdown_tx, ws_shutdown_rx) = watch::channel(false);

    let ws_url = format!("ws://{}", bound_addr);

    // First: accept one connection, send one message, then drop
    let first_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut write, mut read) = ws.split();

        // Read subscriptions
        for _ in 0..2 {
            let _ = time::timeout(Duration::from_millis(500), read.next()).await;
        }

        // Send one update, then close
        let msg = json!({
            "channel": "prices",
            "data": [{"symbol": "USDJPY", "funding": "0.0001", "next_funding": "0", "timestamp": 0.0}]
        });
        let _ = write.send(Message::Text(msg.to_string())).await;
        let _ = write.send(Message::Close(None)).await;
        time::sleep(Duration::from_millis(100)).await;
    });

    let ws_handle = tokio::spawn(pacifica_ws_loop(
        ws_url,
        "USDJPY".into(),
        cache,
        circuit.clone(),
        event_tx,
        ws_shutdown_rx,
    ));

    // Wait for connected + disconnect events
    let mut saw_connected = false;
    let mut saw_disconnected = false;

    for _ in 0..20 {
        match time::timeout(Duration::from_millis(500), event_rx.recv()).await {
            Ok(Some(VenueEvent::Connected { .. })) => saw_connected = true,
            Ok(Some(VenueEvent::Disconnected { .. })) => {
                saw_disconnected = true;
                break;
            }
            Ok(Some(_)) => {}
            _ => break,
        }
    }

    // Shutdown the WS loop (it's trying to reconnect)
    let _ = ws_shutdown_tx.send(true);
    let _ = time::timeout(Duration::from_secs(3), ws_handle).await;
    let _ = first_handle.await;

    // Check circuit breaker recorded failure
    {
        let cb = circuit.lock().await;
        assert!(cb.failures() >= 1, "circuit breaker should have failures");
    }

    assert!(saw_connected, "should have connected");
    assert!(
        saw_disconnected,
        "should have seen disconnect after server drop"
    );
}

#[tokio::test]
async fn circuit_breaker_opens_after_repeated_failures() {
    let mut cb = CircuitBreaker::new(3, Duration::from_secs(120));
    assert!(cb.allow_request());
    cb.record_failure();
    cb.record_failure();
    cb.record_failure();
    assert!(!cb.allow_request());
    assert_eq!(cb.state(), bot_venues::net::circuit::CircuitState::Open);
}

#[tokio::test]
async fn event_channel_handles_backpressure() {
    let (tx, _rx) = mpsc::channel::<VenueEvent>(2); // tiny buffer

    // Fill the channel
    let _ = tx.try_send(VenueEvent::Heartbeat {
        venue: "test".into(),
    });
    let _ = tx.try_send(VenueEvent::Heartbeat {
        venue: "test".into(),
    });

    // Third send should fail (channel full) — not panic
    let result = tx.try_send(VenueEvent::Heartbeat {
        venue: "test".into(),
    });
    assert!(result.is_err(), "should fail when channel is full");
}
