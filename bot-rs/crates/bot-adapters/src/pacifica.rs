//! `PacificaReadOnlyAdapter` — wraps `bot_venues::pacifica::rest::PacificaRest`
//! to implement `VenueAdapter`.
//!
//! This adapter is STRICTLY read-only. No account credential, no signing,
//! no order submission. Dry-run simulation only.

use async_trait::async_trait;
use tracing::{info, warn};

use bot_types::{AnnualizedRate, HourlyRate, Usd, Venue};
use bot_venues::pacifica::rest::PacificaRest;

use crate::venue::{
    AdapterError, FillReport, OrderIntent, PositionView, VenueAdapter, VenueSnapshot,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default Pacifica public REST base URL (from `bot-venues::config`).
pub const PACIFICA_REST_URL: &str = "https://api.pacifica.fi/api/v1";

/// Pacifica funding interval is **1 hour**
/// (<https://docs.pacifica.fi/trading-on-pacifica/funding-rates>).
const FUNDING_INTERVAL_HOURS: f64 = 1.0;
const FUNDING_INTERVAL_SECONDS: i64 = (FUNDING_INTERVAL_HOURS * 3600.0) as i64;

/// Fallback tick size used when the venue metadata endpoint is unavailable.
/// TODO: query from Pacifica instrument metadata endpoint when available.
const FALLBACK_TICK_SIZE: f64 = 0.01;

/// Synthetic depth-curve offsets in basis points (for fractal_delta OLS).
/// Used when the REST orderbook only returns top-of-book.
/// TODO: replace with a multi-level orderbook fetch once Pacifica exposes it.
const SYNTHETIC_DEPTH_OFFSETS_BPS: [f64; 5] = [1.0, 2.0, 5.0, 10.0, 20.0];

/// Demo symbol list returned by `list_symbols` when the venue API doesn't
/// expose a symbol directory endpoint.
///
/// Msg 069 RWA swap (2026-04-15): 7 crypto + 3 RWA.
/// - Removed `OP`, `MATIC`, `APT` — not listed on Pacifica.
/// - Added `XAU`, `XAG`, `PAXG` — Dol's core RWA yield symbols.
/// - XAU/XAG hedge via trade.xyz (HIP-3 on Hyperliquid infrastructure)
///   with coin identifiers `xyz:GOLD` / `xyz:SILVER`.
/// - PAXG has a native HL perp (regular coin id).
///
/// TODO: replace with a real Pacifica instruments endpoint.
const DEMO_SYMBOLS: &[&str] = &[
    "BTC", "ETH", "SOL", "BNB", "ARB", "AVAX", "SUI", "XAU", "XAG", "PAXG",
];

// ── Adapter struct ────────────────────────────────────────────────────────────

/// Read-only Pacifica adapter.
///
/// Wraps `PacificaRest` without an account credential — only public endpoints
/// are used (funding rates, orderbook). No order execution path exists.
pub struct PacificaReadOnlyAdapter {
    rest: PacificaRest,
}

impl PacificaReadOnlyAdapter {
    /// Create a new adapter against the given REST base URL.
    ///
    /// The account is set to an empty string; only public endpoints are hit.
    pub fn new(base_url: impl Into<String>) -> Self {
        let url: String = base_url.into();
        Self {
            rest: PacificaRest::new(&url, ""),
        }
    }

    /// Convenience constructor using the production Pacifica REST URL.
    pub fn production() -> Self {
        Self::new(PACIFICA_REST_URL)
    }
}

// ── VenueAdapter impl ─────────────────────────────────────────────────────────

#[async_trait]
impl VenueAdapter for PacificaReadOnlyAdapter {
    fn venue(&self) -> Venue {
        Venue::Pacifica
    }

    async fn fetch_snapshot(&self, symbol: &str) -> Result<VenueSnapshot, AdapterError> {
        // Fetch funding and orderbook in parallel.
        let (funding_res, book_res) = tokio::try_join!(
            self.rest.get_funding(symbol),
            self.rest.get_orderbook(symbol),
        )
        .map_err(|e| AdapterError::Network(e.to_string()))?;

        // ── Timestamp ─────────────────────────────────────────────────────
        let ts_ms = if book_res.timestamp > 0 {
            book_res.timestamp
        } else {
            // Fall back to wall clock if the REST response has no timestamp.
            chrono::Utc::now().timestamp_millis()
        };

        // ── Prices ────────────────────────────────────────────────────────
        let bid_price = book_res.best_bid.as_ref().map(|l| l.price).unwrap_or(0.0);
        let ask_price = book_res.best_ask.as_ref().map(|l| l.price).unwrap_or(0.0);

        // If either side is missing / zero, mid defaults to the available side.
        let mid_price = if bid_price > 0.0 && ask_price > 0.0 {
            (bid_price + ask_price) / 2.0
        } else if bid_price > 0.0 {
            bid_price
        } else if ask_price > 0.0 {
            ask_price
        } else {
            return Err(AdapterError::Parse(format!(
                "pacifica: both bid and ask are zero for symbol {symbol}"
            )));
        };

        // ── Depth ─────────────────────────────────────────────────────────
        let bid_depth_usd = book_res
            .best_bid
            .as_ref()
            .map(|l| l.price * l.size)
            .unwrap_or(0.0);
        let ask_depth_usd = book_res
            .best_ask
            .as_ref()
            .map(|l| l.price * l.size)
            .unwrap_or(0.0);
        let depth_top_usd = bid_depth_usd + ask_depth_usd;

        // Synthetic depth curve — Pacifica REST /book only returns top-of-book.
        // TODO: fetch multi-level orderbook when Pacifica exposes it, and fit the
        //       real cumulative depth at each offset for fractal_delta OLS.
        let depth_curve: Vec<(f64, f64)> = SYNTHETIC_DEPTH_OFFSETS_BPS
            .iter()
            .map(|&bps| (bps, depth_top_usd))
            .collect();

        // ── Funding ───────────────────────────────────────────────────────
        // `FundingRate.apy_equivalent` = hourly * 365 * 24 (annual fraction).
        let funding_rate_annual = AnnualizedRate(funding_res.apy_equivalent);
        let funding_rate_hourly = HourlyRate(funding_res.apy_equivalent / (365.0 * 24.0));

        // Next funding timestamp: prefer the value from the API; fall back to
        // now + one interval.
        let next_funding_ts_ms = if funding_res.next_timestamp > 0 {
            // Pacifica returns seconds; convert to ms.
            funding_res.next_timestamp * 1000
        } else {
            ts_ms + FUNDING_INTERVAL_SECONDS * 1000
        };

        // ── Volume / OI ───────────────────────────────────────────────────
        // Populated from the `volume_24h` and `open_interest` fields of the
        // Pacifica `/info/prices` PriceInfo response via `FundingRate.volume_24h_usd`
        // and `FundingRate.open_interest_usd` (added in Step B.1).
        let volume_24h_usd = Usd(funding_res.volume_24h_usd);
        let open_interest_usd = Usd(funding_res.open_interest_usd);

        // ── Mark bias ─────────────────────────────────────────────────────
        // TODO: compute (mark - mid) / mid * 1e4 once PacificaRest exposes
        //       the mark price separately from mid. Currently mark == mid on
        //       the normalized FundingRate; bias would always be 0.
        let mark_bias_bps = 0.0;

        // ── Tick size ─────────────────────────────────────────────────────
        // TODO: query from Pacifica instruments/metadata endpoint when available.
        let tick_size = FALLBACK_TICK_SIZE;

        Ok(VenueSnapshot {
            venue: Venue::Pacifica,
            symbol: symbol.to_string(),
            ts_ms,
            mid_price,
            bid_price,
            ask_price,
            tick_size,
            mark_bias_bps,
            depth_top_usd,
            depth_curve,
            funding_rate_annual,
            funding_rate_hourly,
            funding_interval_seconds: FUNDING_INTERVAL_SECONDS,
            next_funding_ts_ms,
            volume_24h_usd,
            open_interest_usd,
        })
    }

    async fn list_symbols(&self) -> Result<Vec<String>, AdapterError> {
        // TODO: replace with a real Pacifica instruments/markets endpoint when available.
        warn!(
            venue = "pacifica",
            "list_symbols: using hardcoded demo list; \
             no Pacifica instruments endpoint is available yet"
        );
        Ok(DEMO_SYMBOLS.iter().map(|s| s.to_string()).collect())
    }

    async fn fetch_position(&self, _symbol: &str) -> Result<Option<PositionView>, AdapterError> {
        // Public-API-only adapter: no account, no positions.
        Ok(None)
    }

    async fn submit_dryrun(&self, order: &OrderIntent) -> Result<FillReport, AdapterError> {
        let ts_ms = chrono::Utc::now().timestamp_millis();

        // Attempt to derive a reference price for the simulated fill.
        // Best effort: fetch current mid; fall back to limit_price.
        let mid_price = match self.rest.get_orderbook(&order.symbol).await {
            Ok(book) => {
                let bid = book.best_bid.map(|l| l.price).unwrap_or(0.0);
                let ask = book.best_ask.map(|l| l.price).unwrap_or(0.0);
                if bid > 0.0 && ask > 0.0 {
                    (bid + ask) / 2.0
                } else {
                    order.limit_price.unwrap_or(0.0)
                }
            }
            Err(_) => order.limit_price.unwrap_or(0.0),
        };

        let avg_fill_price = order.limit_price.unwrap_or(mid_price);

        info!(
            venue = ?order.venue,
            symbol = %order.symbol,
            side = order.side,
            notional_usd = order.notional_usd.0,
            avg_fill_price,
            kind = ?order.kind,
            client_tag = %order.client_tag,
            "[DRY-RUN] would have executed order — no network submission"
        );

        Ok(FillReport {
            order_tag: order.client_tag.clone(),
            venue: order.venue,
            symbol: order.symbol.clone(),
            side: order.side,
            filled_notional_usd: order.notional_usd,
            avg_fill_price,
            realized_slippage_bps: 0.0,
            fees_paid_usd: Usd(0.0),
            ts_ms,
            dry_run: true,
        })
    }
}
