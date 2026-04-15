//! Raw Lighter JSON types. Deserialized from WS and REST responses.

use serde::Deserialize;

// ── REST responses ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LighterResponse<T> {
    pub code: i32,
    pub message: Option<String>,
    #[serde(flatten)]
    pub data: T,
}

// ── Markets (symbol → market_id resolution) ────────────────────

#[derive(Debug, Deserialize)]
pub struct MarketsData {
    pub order_books: Vec<MarketItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketItem {
    pub market_id: u32,
    pub symbol: String,
    pub market_type: Option<serde_json::Value>,
}

// ── Account ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AccountsData {
    pub accounts: Vec<AccountItem>,
}

#[derive(Debug, Deserialize)]
pub struct AccountItem {
    pub index: u32,
    pub l1_address: Option<String>,
    pub available_balance: Option<String>,
    pub collateral: Option<String>,
    pub positions: Option<Vec<PositionItem>>,
}

#[derive(Debug, Deserialize)]
pub struct PositionItem {
    pub market_id: u32,
    pub symbol: Option<String>,
    pub sign: i32, // 1=long, -1=short, 0=flat
    pub position: Option<String>,
    pub avg_entry_price: Option<String>,
    pub position_value: Option<String>,
    pub unrealized_pnl: Option<String>,
}

// ── Funding rates ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FundingRatesData {
    pub funding_rates: Vec<FundingRateItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FundingRateItem {
    pub market_id: u32,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub rate: f64,
}

// ── Orderbook ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OrderbookData {
    pub asks: Vec<OrderItem>,
    pub bids: Vec<OrderItem>,
}

#[derive(Debug, Deserialize)]
pub struct OrderItem {
    pub order_index: Option<u32>,
    pub price: String,
    pub remaining_base_amount: Option<String>,
}

// ── WS messages ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WsMessage {
    pub channel: Option<String>,
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Market stats channel (funding rate).
#[derive(Debug, Deserialize)]
pub struct WsMarketStats {
    pub current_funding_rate: Option<f64>,
    pub funding_rate: Option<f64>,
}

/// Ticker channel (best bid/ask).
#[derive(Debug, Deserialize)]
pub struct WsTicker {
    pub best_bid_price: Option<String>,
    pub best_ask_price: Option<String>,
    pub best_bid_quantity: Option<String>,
    pub best_ask_quantity: Option<String>,
}
