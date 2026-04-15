//! Pacifica WebSocket client — connects to `wss://ws.pacifica.fi/ws`,
//! subscribes to `prices` and `book` channels, and updates cached state.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::rest::{parse_book_data, parse_funding_rate};
use super::types::*;
use crate::event::VenueEvent;
use crate::net::{CircuitBreaker, Reconnect};
use crate::venue::{FundingRate, OrderbookTop};

/// Ping interval — Pacifica closes after 60s idle.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Read timeout — if no data in 2x ping interval, assume dead.
const READ_TIMEOUT: Duration = Duration::from_secs(65);

/// Cached latest state from WS.
#[derive(Debug, Default)]
pub struct WsCache {
    pub funding: Option<FundingRate>,
    pub book: Option<OrderbookTop>,
}

/// Run the Pacifica WS loop as a spawned task.
///
/// Manages connect → subscribe → read → reconnect lifecycle.
/// Updates `cache` on each message. Sends `VenueEvent` to `event_tx`.
pub async fn run_ws_loop(
    ws_url: String,
    symbol: String,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut backoff = Reconnect::default_production();

    loop {
        // Check shutdown before attempting connection
        if *shutdown_rx.borrow() {
            tracing::info!(venue = "pacifica", "ws loop: shutdown signal received");
            break;
        }

        tracing::info!(
            venue = "pacifica",
            url = %ws_url,
            attempt = backoff.attempts(),
            "ws: connecting"
        );

        match connect_and_run(
            &ws_url,
            &symbol,
            &cache,
            &circuit,
            &event_tx,
            &mut shutdown_rx,
        )
        .await
        {
            Ok(()) => {
                // Clean shutdown
                tracing::info!(venue = "pacifica", "ws: clean disconnect");
                break;
            }
            Err(e) => {
                tracing::warn!(venue = "pacifica", error = %e, "ws: disconnected");
                {
                    let mut cb = circuit.lock().await;
                    cb.record_failure();
                }
                let _ = event_tx.try_send(VenueEvent::Disconnected {
                    venue: "pacifica".into(),
                    reason: e.to_string(),
                });
            }
        }

        // Backoff before reconnect
        let delay = backoff.next_delay();
        tracing::info!(
            venue = "pacifica",
            delay_ms = delay.as_millis() as u64,
            "ws: reconnecting after backoff"
        );

        tokio::select! {
            _ = time::sleep(delay) => {},
            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "pacifica", "ws: shutdown during backoff");
                break;
            }
        }
    }
}

async fn connect_and_run(
    ws_url: &str,
    symbol: &str,
    cache: &Arc<Mutex<WsCache>>,
    circuit: &Arc<Mutex<CircuitBreaker>>,
    event_tx: &mpsc::Sender<VenueEvent>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let (ws, _) = connect_async(ws_url).await?;
    let (mut write, mut read) = ws.split();

    // Subscribe to channels
    let sub_prices = json!({
        "method": "subscribe",
        "params": { "source": "prices" }
    });
    let sub_book = json!({
        "method": "subscribe",
        "params": { "source": "book", "symbol": symbol, "agg_level": 1 }
    });

    write.send(Message::Text(sub_prices.to_string())).await?;
    write.send(Message::Text(sub_book.to_string())).await?;

    tracing::info!(
        venue = "pacifica",
        symbol = symbol,
        "ws: subscribed to prices + book"
    );

    // Notify connected
    {
        let mut cb = circuit.lock().await;
        cb.record_success();
    }
    let _ = event_tx.try_send(VenueEvent::Connected {
        venue: "pacifica".into(),
    });

    // Reset backoff on successful connect (caller manages backoff,
    // but we signal via circuit success)

    let mut ping_interval = time::interval(PING_INTERVAL);
    ping_interval.tick().await; // consume first immediate tick

    loop {
        tokio::select! {
            // Read WS message
            msg = time::timeout(READ_TIMEOUT, read.next()) => {
                match msg {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        handle_text_message(
                            &text, symbol, cache, event_tx,
                        ).await;
                    }
                    Ok(Some(Ok(Message::Ping(data)))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Ok(Some(Ok(Message::Close(_)))) => {
                        tracing::info!(venue = "pacifica", "ws: server sent close");
                        return Err(anyhow::anyhow!("server close frame"));
                    }
                    Ok(Some(Ok(_))) => {
                        // Binary, Pong, Frame — ignore
                    }
                    Ok(Some(Err(e))) => {
                        return Err(anyhow::anyhow!("ws read error: {}", e));
                    }
                    Ok(None) => {
                        return Err(anyhow::anyhow!("ws stream ended"));
                    }
                    Err(_) => {
                        return Err(anyhow::anyhow!("ws read timeout ({}s)", READ_TIMEOUT.as_secs()));
                    }
                }
            }

            // Periodic ping
            _ = ping_interval.tick() => {
                let ping = json!({ "method": "ping" });
                if let Err(e) = write.send(Message::Text(ping.to_string())).await {
                    return Err(anyhow::anyhow!("ws ping send failed: {}", e));
                }
                let _ = event_tx.try_send(VenueEvent::Heartbeat {
                    venue: "pacifica".into(),
                });
            }

            // Shutdown signal
            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "pacifica", "ws: shutdown signal");
                let _ = write.send(Message::Close(None)).await;
                return Ok(());
            }
        }
    }
}

async fn handle_text_message(
    text: &str,
    symbol: &str,
    cache: &Arc<Mutex<WsCache>>,
    event_tx: &mpsc::Sender<VenueEvent>,
) {
    // Try to parse as a generic WS message
    let msg: WsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                venue = "pacifica",
                error = %e,
                "ws: failed to parse message"
            );
            return;
        }
    };

    // Pong response — ignore
    if msg.method.as_deref() == Some("pong") {
        return;
    }

    match msg.channel.as_deref() {
        Some("prices") => {
            if let Ok(prices_msg) = serde_json::from_str::<PricesChannelData>(text) {
                if let Some(price) = prices_msg.data.iter().find(|p| p.symbol == symbol) {
                    let fr = parse_funding_rate(price);
                    tracing::info!(
                        venue = "pacifica",
                        symbol = %fr.symbol,
                        rate_per_interval = fr.rate_per_interval,
                        apy = fr.apy_equivalent,
                        "funding rate update"
                    );
                    let event = VenueEvent::FundingUpdate {
                        venue: "pacifica".into(),
                        rate: fr.clone(),
                    };
                    {
                        let mut c = cache.lock().await;
                        c.funding = Some(fr);
                    }
                    let _ = event_tx.try_send(event);
                }
            }
        }
        Some("book") => {
            if let Ok(book_msg) = serde_json::from_str::<BookChannelData>(text) {
                if book_msg.data.s == symbol {
                    let ob = parse_book_data(&book_msg.data);
                    tracing::info!(
                        venue = "pacifica",
                        symbol = %ob.symbol,
                        bid = ob.best_bid.as_ref().map(|b| b.price),
                        ask = ob.best_ask.as_ref().map(|a| a.price),
                        "orderbook update"
                    );
                    let event = VenueEvent::OrderbookUpdate {
                        venue: "pacifica".into(),
                        book: ob.clone(),
                    };
                    {
                        let mut c = cache.lock().await;
                        c.book = Some(ob);
                    }
                    let _ = event_tx.try_send(event);
                }
            }
        }
        _ => {
            // Unknown channel or subscription ack — ignore
        }
    }
}
