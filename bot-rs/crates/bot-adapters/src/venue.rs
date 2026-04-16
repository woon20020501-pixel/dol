//! `VenueAdapter` trait and shared data types.
//!
//! Shapes are derived from `integration-spec.md`:
//! - §2.2  `fair_value_oracle` → `VenueQuote` fields in `VenueSnapshot`
//! - §2.5  `fractal_delta`     → `depth_curve` (≥ 5 log-log points)
//! - §5.1  data contract table → full field set
//!
//! **No live order submission.** `submit_dryrun` is the only order path in Step A.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use bot_types::{AnnualizedRate, HourlyRate, Usd, Venue};

// ─────────────────────────────────────────────────────────────────────────────
// Core snapshot type
// ─────────────────────────────────────────────────────────────────────────────

/// Full per-tick market snapshot from one venue.
///
/// Satisfies the §5.1 data contract table:
/// - `compute_fair_value` VenueQuote: `mid_price`, `ts_ms`, `depth_top_usd`,
///   `funding_rate_annual`, `mark_bias_bps`, `tick_size`
/// - `estimate_fractal_delta`: `depth_curve` (≥ 5 points of `(price_offset_bps, depth_usd)`)
/// - `compute_system_state` LiveInputs: `volume_24h_usd`, `open_interest_usd`,
///   `funding_rate_hourly`, `next_funding_ts_ms`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueSnapshot {
    /// Which DEX this snapshot came from (closed enum — I-VENUE).
    pub venue: Venue,

    /// Trading symbol (e.g. `"BTC"`). Byte-identical across venues for a pair (I-SAME).
    pub symbol: String,

    /// Observation timestamp, Unix milliseconds.
    pub ts_ms: i64,

    // ── Price ──────────────────────────────────────────────────────────────
    /// Mid-price = (best_bid + best_ask) / 2, in USD.
    pub mid_price: f64,

    /// Best bid price, in USD.
    pub bid_price: f64,

    /// Best ask price, in USD.
    pub ask_price: f64,

    /// Minimum price increment for this symbol on this venue, in USD.
    /// Used by `normalize_to_tick(p_star, tick_size)` (§2.2).
    pub tick_size: f64,

    /// Mark-price deviation from mid in basis points: `(mark - mid) / mid * 1e4`.
    /// Zero when mark is unavailable; adapter must add a TODO in that case.
    pub mark_bias_bps: f64,

    // ── Depth ─────────────────────────────────────────────────────────────
    /// Top-of-book combined USD depth = bid_price * bid_size + ask_price * ask_size.
    /// Used as the single `depth` value in the §2.2 VenueQuote.
    pub depth_top_usd: f64,

    /// Log-log depth curve for `estimate_fractal_delta` (§2.5).
    ///
    /// Each element is `(price_offset_bps, cumulative_depth_usd)` where
    /// `price_offset_bps` is the distance from mid in basis points and
    /// `cumulative_depth_usd` is the total USD liquidity available within
    /// that offset on the dominant side.
    ///
    /// Must contain ≥ 5 points with distinct positive offsets.
    /// If the underlying orderbook only returns top-of-book, the adapter
    /// generates a synthetic flat curve — see TODO in `PacificaReadOnlyAdapter`.
    pub depth_curve: Vec<(f64, f64)>,

    // ── Funding ───────────────────────────────────────────────────────────
    /// Funding rate in annualized fraction (e.g. 0.15 = 15% p.a.).
    /// Mapped to `VenueQuote.funding_annual` in §2.2.
    pub funding_rate_annual: AnnualizedRate,

    /// Same rate expressed per hour. Stored as `HourlyRate` for `LiveInputs.funding_rate_h`.
    pub funding_rate_hourly: HourlyRate,

    /// Funding payment interval in seconds (e.g. 28800 for Pacifica 8-hour).
    pub funding_interval_seconds: i64,

    /// Estimated Unix timestamp (ms) of next funding settlement.
    pub next_funding_ts_ms: i64,

    // ── Volume / OI ───────────────────────────────────────────────────────
    /// 24-hour rolling trading volume in USD. Used for slippage model and
    /// `LiveInputs.volume_24h`. Set to `Usd(0.0)` when unavailable (with TODO).
    pub volume_24h_usd: Usd,

    /// Current open interest in USD. Used for slippage model and
    /// `LiveInputs.open_interest`. Set to `Usd(0.0)` when unavailable (with TODO).
    pub open_interest_usd: Usd,
}

// ─────────────────────────────────────────────────────────────────────────────
// Position view
// ─────────────────────────────────────────────────────────────────────────────

/// Read-only view of a position held on a venue.
///
/// In Step A, `PacificaReadOnlyAdapter` always returns `None` because it
/// operates without an account credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionView {
    pub venue: Venue,
    pub symbol: String,
    /// +1 = long, -1 = short, 0 = flat.
    pub side: i8,
    pub notional_usd: Usd,
    pub entry_price: f64,
    pub unrealized_pnl_usd: Usd,
}

// ─────────────────────────────────────────────────────────────────────────────
// Order intent (dry-run only in Step A)
// ─────────────────────────────────────────────────────────────────────────────

/// Proposed order. In Step A only the dryrun path exists — `submit_dryrun`
/// logs "would have executed" and returns a simulated fill. No live signing
/// or network submission occurs anywhere in this crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderIntent {
    pub venue: Venue,
    pub symbol: String,
    /// +1 = buy/long, -1 = sell/short.
    pub side: i8,
    pub notional_usd: Usd,
    /// None for market-like IOC; Some(price) for maker-post or limit IOC.
    pub limit_price: Option<f64>,
    pub kind: OrderKind,
    /// Caller-set tag for fill correlation (I-LOCK audit trail).
    pub client_tag: String,
}

/// Order execution style.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderKind {
    /// Maker post-only (cancel if would cross).
    MakerPost,
    /// IOC taker (cancel residual immediately after partial fill).
    TakerIoc,
}

// ─────────────────────────────────────────────────────────────────────────────
// Fill report
// ─────────────────────────────────────────────────────────────────────────────

/// Simulated or real fill returned by `submit_dryrun`.
///
/// In Step A `dry_run` is always `true`. The real executor (Phase 4) will set
/// it to `false` and populate `realized_slippage_bps` from actual fills for
/// the `SlippageObservation` stream (I-SLIP).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillReport {
    pub order_tag: String,
    pub venue: Venue,
    pub symbol: String,
    /// +1 bought, -1 sold.
    pub side: i8,
    pub filled_notional_usd: Usd,
    pub avg_fill_price: f64,
    /// Basis points of slippage vs. limit / mid. Always 0.0 in dry-run.
    pub realized_slippage_bps: f64,
    /// Fees paid in USD. Always 0.0 in dry-run.
    pub fees_paid_usd: Usd,
    pub ts_ms: i64,
    /// True whenever this fill was simulated (no order reached the network).
    pub dry_run: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("network error: {0}")]
    Network(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("fixture error: {0}")]
    Fixture(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// VenueAdapter trait
// ─────────────────────────────────────────────────────────────────────────────

/// Unified interface that every venue adapter must implement.
///
/// All methods are read-only or dry-run. No live order submission path exists
/// in Step A (aurora-omega demo constraint: no mainnet writes).
///
/// The trait is object-safe: `Arc<dyn VenueAdapter>` is valid.
#[async_trait]
pub trait VenueAdapter: Send + Sync {
    /// Which DEX this adapter represents.
    fn venue(&self) -> Venue;

    /// Fetch a full market snapshot for `symbol`.
    ///
    /// May issue multiple parallel network calls internally (funding + orderbook).
    /// Always returns a `VenueSnapshot` with `ts_ms` set to the observation time.
    async fn fetch_snapshot(&self, symbol: &str) -> Result<VenueSnapshot, AdapterError>;

    /// List known symbols for this venue.
    ///
    /// Returns a best-effort list; some adapters hard-code a demo list with a WARN.
    async fn list_symbols(&self) -> Result<Vec<String>, AdapterError>;

    /// Query current position for `symbol` (read-only).
    ///
    /// Returns `Ok(None)` for public-API-only adapters that have no account.
    async fn fetch_position(&self, symbol: &str) -> Result<Option<PositionView>, AdapterError>;

    /// Simulate order submission — logs "would have executed" and returns a
    /// synthetic fill. No network order is submitted. `FillReport.dry_run == true`.
    async fn submit_dryrun(&self, order: &OrderIntent) -> Result<FillReport, AdapterError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Object-safety check (compile-time)
// ─────────────────────────────────────────────────────────────────────────────

/// Verify `VenueAdapter` is object-safe at compile time.
#[allow(dead_code)]
fn _assert_object_safe(_: Arc<dyn VenueAdapter>) {}
