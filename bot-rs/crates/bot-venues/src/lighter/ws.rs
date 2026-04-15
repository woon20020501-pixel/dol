//! Lighter WebSocket client — connects to
//! `wss://mainnet.zklighter.elliot.ai/stream`, subscribes to
//! `market_stats/{marketId}` and `ticker/{marketId}`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::rest::parse_funding_rate;
use super::types::*;
use crate::event::VenueEvent;
use crate::net::{CircuitBreaker, Reconnect};
use crate::venue::{FundingRate, OrderbookLevel, OrderbookTop};

/// Lighter uses native WS ping, 90s interval.
/// Server closes after 120s idle.
const PING_INTERVAL: Duration = Duration::from_secs(90);

/// Read timeout — 2x ping.
const READ_TIMEOUT: Duration = Duration::from_secs(185);

#[derive(Debug, Default)]
pub struct WsCache {
    pub funding: Option<FundingRate>,
    pub book: Option<OrderbookTop>,
}

pub async fn run_ws_loop(
    ws_url: String,
    symbol: String,
    market_id: u32,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut backoff = Reconnect::default_production();

    loop {
        if *shutdown_rx.borrow() {
            tracing::info!(venue = "lighter", "ws loop: shutdown signal received");
            break;
        }

        tracing::info!(
            venue = "lighter",
            url = %ws_url,
            market_id = market_id,
            attempt = backoff.attempts(),
            "ws: connecting"
        );

        match connect_and_run(
            &ws_url,
            &symbol,
            market_id,
            &cache,
            &circuit,
            &event_tx,
            &mut shutdown_rx,
        )
        .await
        {
            Ok(()) => {
                tracing::info!(venue = "lighter", "ws: clean disconnect");
                break;
            }
            Err(e) => {
                tracing::warn!(venue = "lighter", error = %e, "ws: disconnected");
                {
                    let mut cb = circuit.lock().await;
                    cb.record_failure();
                }
                let _ = event_tx.try_send(VenueEvent::Disconnected {
                    venue: "lighter".into(),
                    reason: e.to_string(),
                });
            }
        }

        let delay = backoff.next_delay();
        tracing::info!(
            venue = "lighter",
            delay_ms = delay.as_millis() as u64,
            "ws: reconnecting after backoff"
        );

        tokio::select! {
            _ = time::sleep(delay) => {},
            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "lighter", "ws: shutdown during backoff");
                break;
            }
        }
    }
}

async fn connect_and_run(
    ws_url: &str,
    symbol: &str,
    market_id: u32,
    cache: &Arc<Mutex<WsCache>>,
    circuit: &Arc<Mutex<CircuitBreaker>>,
    event_tx: &mpsc::Sender<VenueEvent>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let (ws, _) = connect_async(ws_url).await?;
    let (mut write, mut read) = ws.split();

    // Subscribe
    let sub_stats = json!({
        "type": "subscribe",
        "channel": format!("market_stats/{}", market_id)
    });
    let sub_ticker = json!({
        "type": "subscribe",
        "channel": format!("ticker/{}", market_id)
    });

    write.send(Message::Text(sub_stats.to_string())).await?;
    write.send(Message::Text(sub_ticker.to_string())).await?;

    tracing::info!(
        venue = "lighter",
        symbol = symbol,
        market_id = market_id,
        "ws: subscribed to market_stats + ticker"
    );

    {
        let mut cb = circuit.lock().await;
        cb.record_success();
    }
    let _ = event_tx.try_send(VenueEvent::Connected {
        venue: "lighter".into(),
    });

    let mut ping_interval = time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    loop {
        tokio::select! {
            msg = time::timeout(READ_TIMEOUT, read.next()) => {
                match msg {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        handle_text_message(
                            &text, symbol, market_id, cache, event_tx,
                        ).await;
                    }
                    Ok(Some(Ok(Message::Ping(data)))) => {
                        let _ = write.send(Message::Pong(data)).await;
                    }
                    Ok(Some(Ok(Message::Close(_)))) => {
                        return Err(anyhow::anyhow!("server close frame"));
                    }
                    Ok(Some(Ok(_))) => {}
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

            _ = ping_interval.tick() => {
                // Lighter uses native WS ping frame
                if let Err(e) = write.send(Message::Ping(vec![])).await {
                    return Err(anyhow::anyhow!("ws ping failed: {}", e));
                }
                let _ = event_tx.try_send(VenueEvent::Heartbeat {
                    venue: "lighter".into(),
                });
            }

            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "lighter", "ws: shutdown signal");
                let _ = write.send(Message::Close(None)).await;
                return Ok(());
            }
        }
    }
}

async fn handle_text_message(
    text: &str,
    symbol: &str,
    market_id: u32,
    cache: &Arc<Mutex<WsCache>>,
    event_tx: &mpsc::Sender<VenueEvent>,
) {
    let msg: WsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(venue = "lighter", error = %e, "ws: parse failed");
            return;
        }
    };

    let channel = match msg.channel.as_deref() {
        Some(c) => c,
        None => return,
    };

    let stats_channel = format!("market_stats/{}", market_id);
    let ticker_channel = format!("ticker/{}", market_id);

    if channel == stats_channel {
        // Funding rate
        if let Ok(stats) = serde_json::from_value::<WsMarketStats>(msg.data.clone()) {
            let rate = stats
                .current_funding_rate
                .or(stats.funding_rate)
                .unwrap_or(0.0);
            let item = FundingRateItem {
                market_id,
                exchange: Some("lighter".into()),
                symbol: Some(symbol.into()),
                rate,
            };
            let fr = parse_funding_rate(&item, symbol);
            tracing::info!(
                venue = "lighter",
                symbol = symbol,
                rate_per_interval = fr.rate_per_interval,
                apy = fr.apy_equivalent,
                "funding rate update"
            );
            let event = VenueEvent::FundingUpdate {
                venue: "lighter".into(),
                rate: fr.clone(),
            };
            {
                let mut c = cache.lock().await;
                c.funding = Some(fr);
            }
            let _ = event_tx.try_send(event);
        }
    } else if channel == ticker_channel {
        // Orderbook top
        if let Ok(ticker) = serde_json::from_value::<WsTicker>(msg.data.clone()) {
            let best_bid = ticker.best_bid_price.as_ref().map(|p| OrderbookLevel {
                price: p.parse().unwrap_or(0.0),
                size: ticker
                    .best_bid_quantity
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
                orders: 1,
            });
            let best_ask = ticker.best_ask_price.as_ref().map(|p| OrderbookLevel {
                price: p.parse().unwrap_or(0.0),
                size: ticker
                    .best_ask_quantity
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
                orders: 1,
            });
            let ob = OrderbookTop {
                symbol: symbol.to_string(),
                best_bid,
                best_ask,
                timestamp: 0,
            };
            tracing::info!(
                venue = "lighter",
                symbol = symbol,
                bid = ob.best_bid.as_ref().map(|b| b.price),
                ask = ob.best_ask.as_ref().map(|a| a.price),
                "orderbook update"
            );
            let event = VenueEvent::OrderbookUpdate {
                venue: "lighter".into(),
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
