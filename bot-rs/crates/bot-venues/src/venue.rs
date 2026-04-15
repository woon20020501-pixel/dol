//! Venue trait and shared data types. Mirrors the TS `IVenue`
//! interface from `packages/bot/src/venues/types.ts`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionSide {
    Long,
    Short,
    Flat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub equity: f64,
    pub available: f64,
    pub margin_used: f64,
    pub open_orders: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,
    pub size: f64,
    pub notional_usd: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRate {
    pub symbol: String,
    pub rate_per_interval: f64,
    pub interval_hours: f64,
    pub apy_equivalent: f64,
    pub next_timestamp: i64,
    /// 24-hour rolling trading volume in USD.
    /// Populated from the `volume_24h` field of the Pacifica `/info/prices` PriceInfo response.
    /// Zero when the API returns `null` or the field is absent.
    pub volume_24h_usd: f64,
    /// Current open interest in USD.
    /// Populated from the `open_interest` field of the Pacifica `/info/prices` PriceInfo response.
    /// Zero when the API returns `null` or the field is absent.
    pub open_interest_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub size: f64,
    pub orders: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookTop {
    pub symbol: String,
    pub best_bid: Option<OrderbookLevel>,
    pub best_ask: Option<OrderbookLevel>,
    pub timestamp: i64,
}

/// Unified venue interface. M2 implements for Pacifica + Lighter.
/// Write methods (order execution) land in M5.
#[async_trait]
pub trait Venue: Send + Sync {
    fn name(&self) -> &str;

    /// Start WS connection and background tasks.
    async fn connect(&mut self) -> anyhow::Result<()>;

    /// Graceful shutdown: drain pending, send close frame.
    async fn disconnect(&mut self) -> anyhow::Result<()>;

    async fn get_balance(&self) -> anyhow::Result<Balance>;
    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>>;
    async fn get_funding(&self, symbol: &str) -> anyhow::Result<FundingRate>;
    async fn get_orderbook(&self, symbol: &str) -> anyhow::Result<OrderbookTop>;
}
