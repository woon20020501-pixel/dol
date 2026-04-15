//! Raw Pacifica JSON types. Deserialized from WS and REST responses.
//! Converted to normalized `venue::*` types by the adapter.

use serde::Deserialize;

// ── REST API envelope ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: T,
    pub error: Option<String>,
    pub code: Option<serde_json::Value>,
}

// ── Account / Balance ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AccountData {
    pub balance: String,
    pub account_equity: String,
    pub available_to_spend: String,
    pub total_margin_used: String,
    pub positions_count: Option<u32>,
    pub orders_count: Option<u32>,
}

// ── Positions ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PositionItem {
    pub symbol: String,
    pub side: String, // "bid" = long, "ask" = short
    pub amount: String,
    pub entry_price: String,
    pub funding: Option<String>, // unrealized PnL field
}

// ── Price info (funding rates) ─────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PriceInfo {
    pub symbol: String,
    pub funding: String, // hourly rate
    pub next_funding: Option<String>,
    pub mark: Option<String>,
    pub mid: Option<String>,
    pub oracle: Option<String>,
    pub open_interest: Option<String>,
    pub volume_24h: Option<String>,
    pub timestamp: Option<f64>, // unix seconds
}

// ── Orderbook ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct BookLevel {
    pub p: String,      // price
    pub a: String,      // amount
    pub n: Option<u32>, // order count
}

#[derive(Debug, Clone, Deserialize)]
pub struct BookData {
    pub s: String,                           // symbol
    pub l: (Vec<BookLevel>, Vec<BookLevel>), // (bids, asks)
    pub t: Option<i64>,                      // timestamp ms
}

// ── WS messages ────────────────────────────────────────────────

/// Top-level WS message. We dispatch on `channel` or `method`.
#[derive(Debug, Deserialize)]
pub struct WsMessage {
    pub channel: Option<String>,
    pub method: Option<String>,
    pub data: Option<serde_json::Value>,
}

/// Parsed prices channel message.
#[derive(Debug, Deserialize)]
pub struct PricesChannelData {
    pub channel: String,
    pub data: Vec<PriceInfo>,
}

/// Parsed book channel message.
#[derive(Debug, Deserialize)]
pub struct BookChannelData {
    pub channel: String,
    pub data: BookData,
}
