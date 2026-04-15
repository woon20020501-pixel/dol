//! Pacifica venue adapter — WS + REST with circuit breaker fallback.

pub mod rest;
pub mod types;
pub mod ws;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch, Mutex};

use crate::config::PacificaConfig;
use crate::event::VenueEvent;
use crate::net::CircuitBreaker;
use crate::venue::{Balance, FundingRate, OrderbookTop, Position, Venue};

use self::rest::PacificaRest;
use self::ws::WsCache;

/// REST fallback polling interval when circuit is open.
const REST_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Event channel capacity.
const EVENT_CHANNEL_SIZE: usize = 256;

pub struct PacificaVenue {
    config: PacificaConfig,
    rest: PacificaRest,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    event_rx: Option<mpsc::Receiver<VenueEvent>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    ws_handle: Option<tokio::task::JoinHandle<()>>,
    rest_poll_handle: Option<tokio::task::JoinHandle<()>>,
}

impl PacificaVenue {
    pub fn new(config: PacificaConfig) -> Self {
        let rest = PacificaRest::new(&config.rest_url, &config.account);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_SIZE);

        Self {
            config,
            rest,
            cache: Arc::new(Mutex::new(WsCache::default())),
            circuit: Arc::new(Mutex::new(CircuitBreaker::default_production())),
            event_tx,
            event_rx: Some(event_rx),
            shutdown_tx: None,
            ws_handle: None,
            rest_poll_handle: None,
        }
    }

    /// Take the event receiver. Can only be called once.
    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<VenueEvent>> {
        self.event_rx.take()
    }
}

#[async_trait]
impl Venue for PacificaVenue {
    fn name(&self) -> &str {
        "pacifica"
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.shutdown_tx.is_some() {
            anyhow::bail!("pacifica: already connected");
        }

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
            cache.clone(),
            circuit.clone(),
            event_tx.clone(),
            shutdown_rx.clone(),
        )));

        // Spawn REST fallback poll loop
        let rest_url = self.config.rest_url.clone();
        let account = self.config.account.clone();
        let rest_shutdown_rx = shutdown_rx;

        self.rest_poll_handle = Some(tokio::spawn(run_rest_poll(
            rest_url,
            account,
            symbol,
            cache,
            circuit,
            event_tx,
            rest_shutdown_rx,
        )));

        tracing::info!(venue = "pacifica", "adapter started");
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        // Wait for tasks to finish (max 2s)
        if let Some(h) = self.ws_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
        if let Some(h) = self.rest_poll_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }

        tracing::info!(venue = "pacifica", "adapter stopped");
        Ok(())
    }

    async fn get_balance(&self) -> anyhow::Result<Balance> {
        self.rest.get_balance().await
    }

    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>> {
        self.rest.get_position(symbol).await
    }

    async fn get_funding(&self, symbol: &str) -> anyhow::Result<FundingRate> {
        // Prefer cached WS data
        {
            let c = self.cache.lock().await;
            if let Some(ref fr) = c.funding {
                if fr.symbol == symbol {
                    return Ok(fr.clone());
                }
            }
        }
        // Fallback to REST
        self.rest.get_funding(symbol).await
    }

    async fn get_orderbook(&self, symbol: &str) -> anyhow::Result<OrderbookTop> {
        // Prefer cached WS data
        {
            let c = self.cache.lock().await;
            if let Some(ref ob) = c.book {
                if ob.symbol == symbol {
                    return Ok(ob.clone());
                }
            }
        }
        // Fallback to REST
        self.rest.get_orderbook(symbol).await
    }
}

/// REST fallback poll — only active when circuit breaker is open.
async fn run_rest_poll(
    rest_url: String,
    account: String,
    symbol: String,
    cache: Arc<Mutex<WsCache>>,
    circuit: Arc<Mutex<CircuitBreaker>>,
    event_tx: mpsc::Sender<VenueEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let rest = PacificaRest::new(&rest_url, &account);
    let mut interval = tokio::time::interval(REST_POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown_rx.changed() => {
                tracing::info!(venue = "pacifica", "rest poll: shutdown");
                return;
            }
        }

        // Only poll when circuit is open (WS is down)
        let should_poll = {
            let mut cb = circuit.lock().await;
            !cb.allow_request()
        };
        // Invert: allow_request() true means circuit closed/half-open (WS trying).
        // We poll REST when circuit is OPEN (allow_request = false).
        if !should_poll {
            continue;
        }

        tracing::debug!(venue = "pacifica", "rest poll: fetching funding + book");

        // Fetch funding
        match rest.get_funding(&symbol).await {
            Ok(fr) => {
                let event = VenueEvent::FundingUpdate {
                    venue: "pacifica".into(),
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
                tracing::warn!(
                    venue = "pacifica",
                    error = %e,
                    "rest poll: funding fetch failed"
                );
            }
        }

        // Fetch orderbook
        match rest.get_orderbook(&symbol).await {
            Ok(ob) => {
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
            Err(e) => {
                tracing::warn!(
                    venue = "pacifica",
                    error = %e,
                    "rest poll: orderbook fetch failed"
                );
            }
        }
    }
}
