//! Lighter venue adapter — WS + REST with circuit breaker fallback.

pub mod rest;
pub mod types;
pub mod ws;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch, Mutex};

use crate::config::LighterConfig;
use crate::event::VenueEvent;
use crate::net::CircuitBreaker;
use crate::venue::{Balance, FundingRate, OrderbookTop, Position, Venue};

use self::rest::LighterRest;
use self::ws::WsCache;

const REST_POLL_INTERVAL: Duration = Duration::from_secs(10);
const EVENT_CHANNEL_SIZE: usize = 256;

pub struct LighterVenue {
    config: LighterConfig,
    rest: LighterRest,
    market_id: Option<u32>,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    event_rx: Option<mpsc::Receiver<VenueEvent>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    ws_handle: Option<tokio::task::JoinHandle<()>>,
    rest_poll_handle: Option<tokio::task::JoinHandle<()>>,
}

impl LighterVenue {
    pub fn new(config: LighterConfig) -> Self {
        let rest = LighterRest::new(
            &config.rest_url,
            config.account_index.as_deref(),
            config.l1_address.as_deref(),
        );
        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_SIZE);

        Self {
            config,
            rest,
            market_id: None,
            cache: Arc::new(Mutex::new(WsCache::default())),
            circuit: Arc::new(Mutex::new(CircuitBreaker::default_production())),
            event_tx,
            event_rx: Some(event_rx),
            shutdown_tx: None,
            ws_handle: None,
            rest_poll_handle: None,
        }
    }

    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<VenueEvent>> {
        self.event_rx.take()
    }

    /// Resolve and cache market_id. Must be called before WS connect.
    async fn ensure_market_id(&mut self) -> anyhow::Result<u32> {
        if let Some(id) = self.market_id {
            return Ok(id);
        }
        let id = self.rest.resolve_market_id(&self.config.symbol).await?;
        tracing::info!(
            venue = "lighter",
            symbol = %self.config.symbol,
            market_id = id,
            "resolved market_id"
        );
        self.market_id = Some(id);
        Ok(id)
    }
}

#[async_trait]
impl Venue for LighterVenue {
    fn name(&self) -> &str {
        "lighter"
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.shutdown_tx.is_some() {
            anyhow::bail!("lighter: already connected");
        }

        let market_id = self.ensure_market_id().await?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        // Spawn WS loop
        let ws_url = self.config.ws_url.clone();
        let symbol = self.config.symbol.clone();
        let cache = Arc::clone(&self.cache);
        let circuit = Arc::clone(&self.circuit);
        let event_tx = self.event_tx.clone();

        self.ws_handle = Some(tokio::spawn(ws::run_ws_loop(
            ws_url,
            symbol.clone(),
            market_id,
            cache.clone(),
            circuit.clone(),
            event_tx.clone(),
            shutdown_rx.clone(),
        )));

        // Spawn REST fallback poll
        let rest_url = self.config.rest_url.clone();
        let account_index = self.config.account_index.clone();
        let l1_address = self.config.l1_address.clone();

        self.rest_poll_handle = Some(tokio::spawn(run_rest_poll(
            rest_url,
            account_index,
            l1_address,
            symbol,
            market_id,
            cache,
            circuit,
            event_tx,
            shutdown_rx,
        )));

        tracing::info!(venue = "lighter", "adapter started");
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(h) = self.ws_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
        if let Some(h) = self.rest_poll_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
        tracing::info!(venue = "lighter", "adapter stopped");
        Ok(())
    }

    async fn get_balance(&self) -> anyhow::Result<Balance> {
        self.rest.get_balance().await
    }

    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>> {
        let mid = self
            .market_id
            .ok_or_else(|| anyhow::anyhow!("lighter: market_id not resolved"))?;
        self.rest.get_position(symbol, mid).await
    }

    async fn get_funding(&self, symbol: &str) -> anyhow::Result<FundingRate> {
        {
            let c = self.cache.lock().await;
            if let Some(ref fr) = c.funding {
                if fr.symbol == symbol {
                    return Ok(fr.clone());
                }
            }
        }
        self.rest.get_funding(symbol).await
    }

    async fn get_orderbook(&self, symbol: &str) -> anyhow::Result<OrderbookTop> {
        {
            let c = self.cache.lock().await;
            if let Some(ref ob) = c.book {
                if ob.symbol == symbol {
                    return Ok(ob.clone());
                }
            }
        }
        let mid = self
            .market_id
            .ok_or_else(|| anyhow::anyhow!("lighter: market_id not resolved"))?;
        self.rest.get_orderbook(mid, symbol).await
    }
}

// Each argument is a distinct connection parameter; bundling into a struct
// would complicate the async-spawn call site without clarity benefit.
#[allow(clippy::too_many_arguments)]
async fn run_rest_poll(
    rest_url: String,
    account_index: Option<String>,
    l1_address: Option<String>,
    symbol: String,
    market_id: u32,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let rest = LighterRest::new(&rest_url, account_index.as_deref(), l1_address.as_deref());
    let mut interval = tokio::time::interval(REST_POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "lighter", "rest poll: shutdown");
                return;
            }
        }

        let should_poll = {
            let mut cb = circuit.lock().await;
            !cb.allow_request()
        };
        if !should_poll {
            continue;
        }

        tracing::debug!(venue = "lighter", "rest poll: fetching funding + book");

        match rest.get_funding(&symbol).await {
            Ok(fr) => {
                let event = VenueEvent::FundingUpdate {
                    venue: "lighter".into(),
                    rate: fr.clone(),
                };
                {
                    let mut c = cache.lock().await;
                    c.funding = Some(fr);
                }
                let _ = event_tx.try_send(event);
                let mut cb = circuit.lock().await;
                cb.record_success();
            }
            Err(e) => {
                tracing::warn!(venue = "lighter", error = %e, "rest poll: funding failed");
            }
        }

        match rest.get_orderbook(market_id, &symbol).await {
            Ok(ob) => {
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
            Err(e) => {
                tracing::warn!(venue = "lighter", error = %e, "rest poll: orderbook failed");
            }
        }
    }
}
