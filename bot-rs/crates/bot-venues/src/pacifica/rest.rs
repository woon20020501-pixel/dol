//! Pacifica REST client — fallback when WS circuit is open.

use anyhow::{Context, Result};
use reqwest::Client;

use super::types::*;
use crate::venue::{Balance, FundingRate, OrderbookLevel, OrderbookTop, Position, PositionSide};

pub struct PacificaRest {
    client: Client,
    base_url: String,
    account: String,
}

/// Pacifica uses **1-hour** funding intervals per
/// <https://docs.pacifica.fi/trading-on-pacifica/funding-rates>.
/// Funding is sampled every 5s via TWAP and settled at each 1h boundary.
/// Per-hour funding is capped at ±4%.
const INTERVAL_HOURS: f64 = 1.0;

impl PacificaRest {
    pub fn new(base_url: &str, account: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            account: account.to_string(),
        }
    }

    pub async fn get_balance(&self) -> Result<Balance> {
        let url = format!("{}/account?account={}", self.base_url, self.account);
        let resp: ApiResponse<AccountData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("pacifica REST /account")?
            .json()
            .await
            .context("pacifica REST /account json")?;

        if !resp.success {
            anyhow::bail!(
                "pacifica /account failed: {}",
                resp.error.unwrap_or_default()
            );
        }

        let d = &resp.data;
        Ok(Balance {
            equity: d.account_equity.parse().unwrap_or(0.0),
            available: d.available_to_spend.parse().unwrap_or(0.0),
            margin_used: d.total_margin_used.parse().unwrap_or(0.0),
            open_orders: d.orders_count.unwrap_or(0),
        })
    }

    pub async fn get_position(&self, symbol: &str) -> Result<Option<Position>> {
        let url = format!("{}/positions?account={}", self.base_url, self.account);
        let resp: ApiResponse<Vec<PositionItem>> = self
            .client
            .get(&url)
            .send()
            .await
            .context("pacifica REST /positions")?
            .json()
            .await
            .context("pacifica REST /positions json")?;

        if !resp.success {
            anyhow::bail!(
                "pacifica /positions failed: {}",
                resp.error.unwrap_or_default()
            );
        }

        let pos = resp.data.iter().find(|p| p.symbol == symbol);
        Ok(pos.map(|p| {
            let side = match p.side.as_str() {
                "bid" => PositionSide::Long,
                "ask" => PositionSide::Short,
                _ => PositionSide::Flat,
            };
            let size: f64 = p.amount.parse().unwrap_or(0.0);
            let entry: f64 = p.entry_price.parse().unwrap_or(0.0);
            Position {
                symbol: p.symbol.clone(),
                side,
                size: size.abs(),
                notional_usd: size.abs() * entry,
                entry_price: entry,
                unrealized_pnl: p
                    .funding
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
            }
        }))
    }

    pub async fn get_funding(&self, symbol: &str) -> Result<FundingRate> {
        let url = format!("{}/info/prices", self.base_url);
        let resp: ApiResponse<Vec<PriceInfo>> = self
            .client
            .get(&url)
            .send()
            .await
            .context("pacifica REST /info/prices")?
            .json()
            .await
            .context("pacifica REST /info/prices json")?;

        if !resp.success {
            anyhow::bail!(
                "pacifica /info/prices failed: {}",
                resp.error.unwrap_or_default()
            );
        }

        let price = resp
            .data
            .iter()
            .find(|p| p.symbol == symbol)
            .ok_or_else(|| anyhow::anyhow!("symbol {} not found in prices", symbol))?;

        Ok(parse_funding_rate(price))
    }

    pub async fn get_orderbook(&self, symbol: &str) -> Result<OrderbookTop> {
        let url = format!("{}/book?symbol={}", self.base_url, symbol);
        let resp: ApiResponse<BookData> = self
            .client
            .get(&url)
            .send()
            .await
            .context("pacifica REST /book")?
            .json()
            .await
            .context("pacifica REST /book json")?;

        if !resp.success {
            anyhow::bail!("pacifica /book failed: {}", resp.error.unwrap_or_default());
        }

        Ok(parse_book_data(&resp.data))
    }
}

// ── Shared parsers (used by both REST and WS) ──────────────────

pub fn parse_funding_rate(p: &PriceInfo) -> FundingRate {
    let hourly: f64 = p.funding.parse().unwrap_or(0.0);
    let next_ts: i64 = p
        .next_funding
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let volume_24h_usd: f64 = p
        .volume_24h
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    let open_interest_usd: f64 = p
        .open_interest
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    FundingRate {
        symbol: p.symbol.clone(),
        rate_per_interval: hourly * INTERVAL_HOURS,
        interval_hours: INTERVAL_HOURS,
        apy_equivalent: hourly * 365.0 * 24.0,
        next_timestamp: next_ts,
        volume_24h_usd,
        open_interest_usd,
    }
}

pub fn parse_book_data(b: &BookData) -> OrderbookTop {
    let best_bid = b.l.0.first().map(|lv| OrderbookLevel {
        price: lv.p.parse().unwrap_or(0.0),
        size: lv.a.parse().unwrap_or(0.0),
        orders: lv.n.unwrap_or(1),
    });
    let best_ask = b.l.1.first().map(|lv| OrderbookLevel {
        price: lv.p.parse().unwrap_or(0.0),
        size: lv.a.parse().unwrap_or(0.0),
        orders: lv.n.unwrap_or(1),
    });

    OrderbookTop {
        symbol: b.s.clone(),
        best_bid,
        best_ask,
        timestamp: b.t.unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_funding_rate_from_price_info() {
        let p = PriceInfo {
            symbol: "USDJPY".into(),
            funding: "0.0001".into(), // hourly
            next_funding: Some("1775000000".into()),
            mark: None,
            mid: None,
            oracle: None,
            open_interest: None,
            volume_24h: None,
            timestamp: Some(1775000000.0),
        };
        let fr = parse_funding_rate(&p);
        assert_eq!(fr.symbol, "USDJPY");
        // Pacifica uses 1h funding intervals, so rate_per_interval == hourly.
        assert!((fr.rate_per_interval - 0.0001).abs() < 1e-10);
        assert_eq!(fr.interval_hours, 1.0);
        assert!((fr.apy_equivalent - 0.0001 * 365.0 * 24.0).abs() < 1e-6);
        assert_eq!(fr.next_timestamp, 1775000000);
    }

    #[test]
    fn parse_book_data_extracts_best_levels() {
        let b = BookData {
            s: "USDJPY".into(),
            l: (
                vec![BookLevel {
                    p: "152.50".into(),
                    a: "1000".into(),
                    n: Some(3),
                }],
                vec![BookLevel {
                    p: "152.55".into(),
                    a: "500".into(),
                    n: Some(2),
                }],
            ),
            t: Some(1775000000000),
        };
        let ob = parse_book_data(&b);
        assert_eq!(ob.symbol, "USDJPY");
        let bid = ob.best_bid.unwrap();
        assert!((bid.price - 152.50).abs() < 1e-10);
        assert!((bid.size - 1000.0).abs() < 1e-10);
        assert_eq!(bid.orders, 3);
        let ask = ob.best_ask.unwrap();
        assert!((ask.price - 152.55).abs() < 1e-10);
    }

    #[test]
    fn parse_book_data_empty_levels() {
        let b = BookData {
            s: "USDJPY".into(),
            l: (vec![], vec![]),
            t: None,
        };
        let ob = parse_book_data(&b);
        assert!(ob.best_bid.is_none());
        assert!(ob.best_ask.is_none());
        assert_eq!(ob.timestamp, 0);
    }
}
