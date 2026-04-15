//! Lighter REST client — fallback when WS circuit is open.
//! Also used for symbol → market_id resolution (always via REST).

use anyhow::{Context, Result};
use reqwest::Client;

use super::types::*;
use crate::venue::{Balance, FundingRate, OrderbookLevel, OrderbookTop, Position, PositionSide};

/// Lighter uses 1-hour funding intervals.
const INTERVAL_HOURS: f64 = 1.0;

pub struct LighterRest {
    client: Client,
    base_url: String,
    account_index: Option<String>,
    l1_address: Option<String>,
}

impl LighterRest {
    pub fn new(base_url: &str, account_index: Option<&str>, l1_address: Option<&str>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            account_index: account_index.map(|s| s.to_string()),
            l1_address: l1_address.map(|s| s.to_string()),
        }
    }

    /// Resolve symbol → market_id via GET /orderBooks.
    pub async fn resolve_market_id(&self, symbol: &str) -> Result<u32> {
        let url = format!("{}/orderBooks", self.base_url);
        let resp: LighterResponse<MarketsData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("lighter REST /orderBooks")?
            .json()
            .await
            .context("lighter REST /orderBooks json")?;

        resp.data
            .order_books
            .iter()
            .find(|m| m.symbol == symbol)
            .map(|m| m.market_id)
            .ok_or_else(|| anyhow::anyhow!("lighter: symbol {} not found in orderBooks", symbol))
    }

    fn account_query(&self) -> Result<String> {
        if let Some(ref idx) = self.account_index {
            Ok(format!("by=index&value={}", idx))
        } else if let Some(ref addr) = self.l1_address {
            Ok(format!("by=l1_address&value={}", addr))
        } else {
            anyhow::bail!("lighter: no account_index or l1_address configured")
        }
    }

    pub async fn get_balance(&self) -> Result<Balance> {
        let query = self.account_query()?;
        let url = format!("{}/account?{}", self.base_url, query);
        let resp: LighterResponse<AccountsData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("lighter REST /account")?
            .json()
            .await
            .context("lighter REST /account json")?;

        let acct = resp
            .data
            .accounts
            .first()
            .ok_or_else(|| anyhow::anyhow!("lighter: no account found"))?;

        let collateral: f64 = acct
            .collateral
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let available: f64 = acct
            .available_balance
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        Ok(Balance {
            equity: collateral,
            available,
            margin_used: (collateral - available).max(0.0),
            open_orders: 0,
        })
    }

    pub async fn get_position(&self, symbol: &str, market_id: u32) -> Result<Option<Position>> {
        let query = self.account_query()?;
        let url = format!("{}/account?{}", self.base_url, query);
        let resp: LighterResponse<AccountsData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("lighter REST /account positions")?
            .json()
            .await
            .context("lighter REST /account positions json")?;

        let acct = resp
            .data
            .accounts
            .first()
            .ok_or_else(|| anyhow::anyhow!("lighter: no account found"))?;

        let pos = acct
            .positions
            .as_ref()
            .and_then(|ps| ps.iter().find(|p| p.market_id == market_id));

        Ok(pos.and_then(|p| parse_position(p, symbol)))
    }

    pub async fn get_funding(&self, symbol: &str) -> Result<FundingRate> {
        let url = format!("{}/funding-rates", self.base_url);
        let resp: LighterResponse<FundingRatesData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("lighter REST /funding-rates")?
            .json()
            .await
            .context("lighter REST /funding-rates json")?;

        let item = resp
            .data
            .funding_rates
            .iter()
            .find(|f| {
                f.symbol.as_deref() == Some(symbol) || f.exchange.as_deref() == Some("lighter")
            })
            .ok_or_else(|| anyhow::anyhow!("lighter: funding rate not found for {}", symbol))?;

        Ok(parse_funding_rate(item, symbol))
    }

    pub async fn get_orderbook(&self, market_id: u32, symbol: &str) -> Result<OrderbookTop> {
        let url = format!(
            "{}/orderBookOrders?market_id={}&limit=1",
            self.base_url, market_id
        );
        let resp: LighterResponse<OrderbookData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("lighter REST /orderBookOrders")?
            .json()
            .await
            .context("lighter REST /orderBookOrders json")?;

        Ok(parse_orderbook(&resp.data, symbol))
    }
}

// ── Shared parsers ─────────────────────────────────────────────

fn parse_position(p: &PositionItem, symbol: &str) -> Option<Position> {
    if p.sign == 0 {
        return None;
    }
    let side = if p.sign > 0 {
        PositionSide::Long
    } else {
        PositionSide::Short
    };
    let size: f64 = p
        .position
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let entry: f64 = p
        .avg_entry_price
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let pnl: f64 = p
        .unrealized_pnl
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    Some(Position {
        symbol: symbol.to_string(),
        side,
        size: size.abs(),
        notional_usd: p
            .position_value
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(size.abs() * entry),
        entry_price: entry,
        unrealized_pnl: pnl,
    })
}

pub fn parse_funding_rate(item: &FundingRateItem, symbol: &str) -> FundingRate {
    let hourly = item.rate;
    FundingRate {
        symbol: symbol.to_string(),
        rate_per_interval: hourly, // 1h interval
        interval_hours: INTERVAL_HOURS,
        apy_equivalent: hourly * 365.0 * 24.0,
        next_timestamp: 0, // Lighter doesn't provide next_funding in this endpoint
        volume_24h_usd: 0.0, // Lighter funding endpoint doesn't surface volume
        open_interest_usd: 0.0, // Lighter funding endpoint doesn't surface OI
    }
}

pub fn parse_orderbook(data: &OrderbookData, symbol: &str) -> OrderbookTop {
    let best_bid = data.bids.first().map(|o| OrderbookLevel {
        price: o.price.parse().unwrap_or(0.0),
        size: o
            .remaining_base_amount
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        orders: 1,
    });
    let best_ask = data.asks.first().map(|o| OrderbookLevel {
        price: o.price.parse().unwrap_or(0.0),
        size: o
            .remaining_base_amount
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        orders: 1,
    });

    OrderbookTop {
        symbol: symbol.to_string(),
        best_bid,
        best_ask,
        timestamp: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_funding_rate_hourly() {
        let item = FundingRateItem {
            market_id: 1,
            exchange: Some("lighter".into()),
            symbol: Some("USDJPY".into()),
            rate: 0.00015,
        };
        let fr = parse_funding_rate(&item, "USDJPY");
        assert_eq!(fr.symbol, "USDJPY");
        assert!((fr.rate_per_interval - 0.00015).abs() < 1e-10);
        assert_eq!(fr.interval_hours, 1.0);
        assert!((fr.apy_equivalent - 0.00015 * 365.0 * 24.0).abs() < 1e-6);
    }

    #[test]
    fn parse_orderbook_extracts_best() {
        let data = OrderbookData {
            bids: vec![OrderItem {
                order_index: Some(1),
                price: "152.30".into(),
                remaining_base_amount: Some("500".into()),
            }],
            asks: vec![OrderItem {
                order_index: Some(2),
                price: "152.35".into(),
                remaining_base_amount: Some("300".into()),
            }],
        };
        let ob = parse_orderbook(&data, "USDJPY");
        assert_eq!(ob.symbol, "USDJPY");
        let bid = ob.best_bid.unwrap();
        assert!((bid.price - 152.30).abs() < 1e-10);
        assert!((bid.size - 500.0).abs() < 1e-10);
        let ask = ob.best_ask.unwrap();
        assert!((ask.price - 152.35).abs() < 1e-10);
    }

    #[test]
    fn parse_orderbook_empty() {
        let data = OrderbookData {
            bids: vec![],
            asks: vec![],
        };
        let ob = parse_orderbook(&data, "USDJPY");
        assert!(ob.best_bid.is_none());
        assert!(ob.best_ask.is_none());
    }

    #[test]
    fn parse_position_long() {
        let p = PositionItem {
            market_id: 1,
            symbol: Some("USDJPY".into()),
            sign: 1,
            position: Some("100".into()),
            avg_entry_price: Some("152.00".into()),
            position_value: Some("15200".into()),
            unrealized_pnl: Some("50".into()),
        };
        let pos = parse_position(&p, "USDJPY").unwrap();
        assert_eq!(pos.side, PositionSide::Long);
        assert!((pos.size - 100.0).abs() < 1e-10);
        assert!((pos.entry_price - 152.0).abs() < 1e-10);
        assert!((pos.unrealized_pnl - 50.0).abs() < 1e-10);
    }

    #[test]
    fn parse_position_short() {
        let p = PositionItem {
            market_id: 1,
            symbol: Some("USDJPY".into()),
            sign: -1,
            position: Some("-200".into()),
            avg_entry_price: Some("153.00".into()),
            position_value: Some("30600".into()),
            unrealized_pnl: Some("-10".into()),
        };
        let pos = parse_position(&p, "USDJPY").unwrap();
        assert_eq!(pos.side, PositionSide::Short);
        assert!((pos.size - 200.0).abs() < 1e-10);
    }

    #[test]
    fn parse_position_flat_returns_none() {
        let p = PositionItem {
            market_id: 1,
            symbol: Some("USDJPY".into()),
            sign: 0,
            position: None,
            avg_entry_price: None,
            position_value: None,
            unrealized_pnl: None,
        };
        assert!(parse_position(&p, "USDJPY").is_none());
    }
}
