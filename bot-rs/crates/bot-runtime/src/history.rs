//! Per-symbol funding-rate history buffer.
//!
//! Feeds OU/ADF fit in the decision layer. Separate from NAV tracking so the
//! history grows independently of NAV events.
//!
//! - Per-symbol ring buffer of `(ts_ms, rate)` tuples.
//! - Capacity: 500 samples (slight headroom over ADF's 50-sample floor and
//!   OU's 30-sample floor).
//! - Time-ordered: newest at the back, oldest evicted from the front.

use std::collections::{BTreeMap, VecDeque};

use bot_types::Venue;

/// Capacity of each per-symbol/venue history ring. 500 samples covers both
/// the ADF minimum (T ≥ 50) and a comfortable OU window (T >> 30).
pub const DEFAULT_HISTORY_CAPACITY: usize = 500;

/// Per-symbol history of cross-venue spread observations.
///
/// Keyed by symbol → (venue → ring of (ts_ms, rate)). Exposes convenience
/// accessors that produce the shape `fit_ou`/`fit_drift`/`adf_test` expect.
#[derive(Debug, Default)]
pub struct FundingHistoryRegistry {
    by_symbol: BTreeMap<String, SymbolHistory>,
    capacity: usize,
}

#[derive(Debug, Default)]
pub struct SymbolHistory {
    per_venue: BTreeMap<Venue, VecDeque<(i64, f64)>>,
    /// Cross-venue spread series (max_venue_rate - min_venue_rate) at each tick.
    spread_series: VecDeque<(i64, f64)>,
}

impl FundingHistoryRegistry {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_HISTORY_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity >= 50, "capacity must cover ADF T ≥ 50 floor");
        Self {
            by_symbol: BTreeMap::new(),
            capacity,
        }
    }

    /// Record a full tick's worth of venue observations for `symbol`.
    /// `observations` = (venue, funding_rate_annual, ts_ms) triples.
    /// Also updates the cross-venue spread series used by ADF/OU fits.
    pub fn record_tick(&mut self, symbol: &str, ts_ms: i64, observations: &[(Venue, f64)]) {
        let entry = self.by_symbol.entry(symbol.to_string()).or_default();

        for &(venue, rate) in observations {
            if !rate.is_finite() {
                continue;
            }
            let venue_q = entry.per_venue.entry(venue).or_default();
            if venue_q.len() >= self.capacity {
                venue_q.pop_front();
            }
            venue_q.push_back((ts_ms, rate));
        }

        // Spread = max - min across observed venues this tick.
        let finite_rates: Vec<f64> = observations
            .iter()
            .filter(|(_, r)| r.is_finite())
            .map(|(_, r)| *r)
            .collect();
        if finite_rates.len() >= 2 {
            let mx = finite_rates
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max);
            let mn = finite_rates.iter().cloned().fold(f64::INFINITY, f64::min);
            if entry.spread_series.len() >= self.capacity {
                entry.spread_series.pop_front();
            }
            entry.spread_series.push_back((ts_ms, mx - mn));
        }
    }

    /// Owned copy of the per-venue history for `symbol`/`venue`.
    /// Returns an empty Vec when no history exists.
    pub fn venue_series(&self, symbol: &str, venue: Venue) -> Vec<(i64, f64)> {
        self.by_symbol
            .get(symbol)
            .and_then(|h| h.per_venue.get(&venue))
            .map(|q| q.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Cross-venue spread history for `symbol`. Shape expected by `fit_ou`
    /// and `fit_drift`.
    pub fn spread_series(&self, symbol: &str) -> Vec<(i64, f64)> {
        self.by_symbol
            .get(symbol)
            .map(|h| h.spread_series.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Raw value-only slice suitable for ADF (expects `&[f64]`).
    pub fn spread_values(&self, symbol: &str) -> Vec<f64> {
        self.by_symbol
            .get(symbol)
            .map(|h| h.spread_series.iter().map(|&(_, v)| v).collect())
            .unwrap_or_default()
    }

    /// Sample count of the spread series.
    pub fn spread_len(&self, symbol: &str) -> usize {
        self.by_symbol
            .get(symbol)
            .map(|h| h.spread_series.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_retrieves_venue_series() {
        let mut reg = FundingHistoryRegistry::new();
        reg.record_tick(
            "BTC",
            1000,
            &[(Venue::Pacifica, 0.04), (Venue::Lighter, 0.03)],
        );
        reg.record_tick(
            "BTC",
            2000,
            &[(Venue::Pacifica, 0.05), (Venue::Lighter, 0.02)],
        );

        let pac = reg.venue_series("BTC", Venue::Pacifica);
        assert_eq!(pac, vec![(1000, 0.04), (2000, 0.05)]);
    }

    #[test]
    fn computes_cross_venue_spread() {
        let mut reg = FundingHistoryRegistry::new();
        reg.record_tick(
            "BTC",
            1000,
            &[(Venue::Pacifica, 0.04), (Venue::Lighter, 0.02)],
        );
        let spread = reg.spread_series("BTC");
        assert_eq!(spread.len(), 1);
        assert!((spread[0].1 - 0.02).abs() < 1e-12);
    }

    #[test]
    fn capacity_enforced() {
        let mut reg = FundingHistoryRegistry::with_capacity(50);
        for i in 0..100 {
            reg.record_tick("BTC", i, &[(Venue::Pacifica, 0.01), (Venue::Lighter, 0.02)]);
        }
        assert_eq!(reg.venue_series("BTC", Venue::Pacifica).len(), 50);
        assert_eq!(reg.spread_len("BTC"), 50);
    }

    #[test]
    fn nan_rates_excluded() {
        let mut reg = FundingHistoryRegistry::new();
        reg.record_tick(
            "BTC",
            1000,
            &[(Venue::Pacifica, f64::NAN), (Venue::Lighter, 0.02)],
        );
        assert_eq!(reg.venue_series("BTC", Venue::Pacifica).len(), 0);
        assert_eq!(reg.venue_series("BTC", Venue::Lighter).len(), 1);
    }

    #[test]
    fn single_venue_tick_has_no_spread() {
        let mut reg = FundingHistoryRegistry::new();
        reg.record_tick("BTC", 1000, &[(Venue::Pacifica, 0.04)]);
        assert_eq!(reg.spread_len("BTC"), 0);
    }

    #[test]
    fn multi_symbol_isolated() {
        let mut reg = FundingHistoryRegistry::new();
        reg.record_tick(
            "BTC",
            1000,
            &[(Venue::Pacifica, 0.04), (Venue::Lighter, 0.02)],
        );
        reg.record_tick(
            "ETH",
            1000,
            &[(Venue::Pacifica, 0.10), (Venue::Lighter, 0.05)],
        );
        assert_eq!(reg.venue_series("BTC", Venue::Pacifica)[0].1, 0.04);
        assert_eq!(reg.venue_series("ETH", Venue::Pacifica)[0].1, 0.10);
    }
}
