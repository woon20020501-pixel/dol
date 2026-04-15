//! `DryRunVenueAdapter` — fixture-replay adapter.
//!
//! Loads `VenueSnapshot` JSON from disk and replays them in order.
//! Useful for end-to-end testing and demo runs that need no network access.
//!
//! # Fixture layout
//!
//! ```text
//! {fixture_dir}/{venue_name}/{symbol}.json
//! ```
//!
//! Each file may contain either:
//! - A single `VenueSnapshot` object, or
//! - A JSON array of `VenueSnapshot` objects (replayed cyclically).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json;
use tracing::info;

use bot_types::{Usd, Venue};

use crate::venue::{
    AdapterError, FillReport, OrderIntent, PositionView, VenueAdapter, VenueSnapshot,
};

// ─────────────────────────────────────────────────────────────────────────────
// Fixture cache entry
// ─────────────────────────────────────────────────────────────────────────────

struct FixtureEntry {
    snapshots: Vec<VenueSnapshot>,
    /// Atomic counter for cyclic replay.
    counter: AtomicUsize,
}

impl FixtureEntry {
    fn next(&self) -> &VenueSnapshot {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.snapshots.len();
        &self.snapshots[idx]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Adapter
// ─────────────────────────────────────────────────────────────────────────────

/// Fixture-replay venue adapter.
///
/// Thread-safe: the atomic counter inside each `FixtureEntry` allows concurrent
/// reads from multiple ticks without a mutex.
pub struct DryRunVenueAdapter {
    venue: Venue,
    fixture_dir: PathBuf,
    /// Lazily-loaded fixtures keyed by upper-case symbol.
    cache: Arc<std::sync::Mutex<HashMap<String, Arc<FixtureEntry>>>>,
}

impl DryRunVenueAdapter {
    /// Create a new adapter for `venue` loading fixtures from `fixture_dir`.
    ///
    /// Fixtures must live at `{fixture_dir}/{venue_name}/{symbol}.json`.
    pub fn new(venue: Venue, fixture_dir: PathBuf) -> Self {
        Self {
            venue,
            fixture_dir,
            cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Venue name as a lowercase string for directory lookup.
    fn venue_dir_name(&self) -> &'static str {
        match self.venue {
            Venue::Pacifica => "pacifica",
            Venue::Hyperliquid => "hyperliquid",
            Venue::Lighter => "lighter",
            Venue::Backpack => "backpack",
        }
    }

    /// Load (or retrieve cached) fixture entry for `symbol`.
    fn load_fixture(&self, symbol: &str) -> Result<Arc<FixtureEntry>, AdapterError> {
        let key = symbol.to_uppercase();

        // Fast path — already cached.
        {
            let guard = self.cache.lock().unwrap();
            if let Some(entry) = guard.get(&key) {
                return Ok(Arc::clone(entry));
            }
        }

        // Load from disk.
        let path = self
            .fixture_dir
            .join(self.venue_dir_name())
            .join(format!("{}.json", symbol));

        let raw = std::fs::read_to_string(&path).map_err(|e| {
            AdapterError::Fixture(format!("cannot read fixture {}: {e}", path.display()))
        })?;

        // Try array first, then single object.
        let snapshots: Vec<VenueSnapshot> = if raw.trim_start().starts_with('[') {
            serde_json::from_str(&raw).map_err(|e| {
                AdapterError::Fixture(format!(
                    "fixture array parse error ({}): {e}",
                    path.display()
                ))
            })?
        } else {
            let single: VenueSnapshot = serde_json::from_str(&raw).map_err(|e| {
                AdapterError::Fixture(format!(
                    "fixture object parse error ({}): {e}",
                    path.display()
                ))
            })?;
            vec![single]
        };

        if snapshots.is_empty() {
            return Err(AdapterError::Fixture(format!(
                "fixture {} contains an empty array",
                path.display()
            )));
        }

        let entry = Arc::new(FixtureEntry {
            snapshots,
            counter: AtomicUsize::new(0),
        });

        // Cache it.
        let mut guard = self.cache.lock().unwrap();
        guard.insert(key, Arc::clone(&entry));
        Ok(entry)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VenueAdapter impl
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl VenueAdapter for DryRunVenueAdapter {
    fn venue(&self) -> Venue {
        self.venue
    }

    async fn fetch_snapshot(&self, symbol: &str) -> Result<VenueSnapshot, AdapterError> {
        let entry = self.load_fixture(symbol)?;
        Ok(entry.next().clone())
    }

    async fn list_symbols(&self) -> Result<Vec<String>, AdapterError> {
        let venue_path = self.fixture_dir.join(self.venue_dir_name());

        let read_dir = std::fs::read_dir(&venue_path).map_err(|e| {
            AdapterError::Fixture(format!(
                "cannot list fixture dir {}: {e}",
                venue_path.display()
            ))
        })?;

        let mut symbols = Vec::new();
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".json") {
                let sym = name_str.trim_end_matches(".json").to_uppercase();
                symbols.push(sym);
            }
        }
        symbols.sort();
        Ok(symbols)
    }

    async fn fetch_position(&self, _symbol: &str) -> Result<Option<PositionView>, AdapterError> {
        // Fixture adapter has no positions.
        Ok(None)
    }

    async fn submit_dryrun(&self, order: &OrderIntent) -> Result<FillReport, AdapterError> {
        // Use the fixture mid price for simulated fill when available.
        let avg_fill_price = if let Ok(entry) = self.load_fixture(&order.symbol) {
            let snap = entry.next();
            order.limit_price.unwrap_or(snap.mid_price)
        } else {
            order.limit_price.unwrap_or(0.0)
        };

        let ts_ms = chrono::Utc::now().timestamp_millis();

        info!(
            venue = ?order.venue,
            symbol = %order.symbol,
            side = order.side,
            notional_usd = order.notional_usd.0,
            avg_fill_price,
            kind = ?order.kind,
            client_tag = %order.client_tag,
            "[DRY-RUN / FIXTURE] would have executed order — no network submission"
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
