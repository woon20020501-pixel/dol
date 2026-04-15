//! Simplified weighted fair-value computation for the demo tick loop.
//!
//! **SIMPLIFIED:** the real framework version is in aurora-omega-1.1.3
//! `strategy/fair_value_oracle.py`. Replace with the full Rust port in v1.
//! This version is weighted-mid only — no staleness exponential decay,
//! no Kalman filter lead/lag tracking, no hard-drop age gate.
//!
//! The full oracle contract is in `integration-spec.md` §2.2.

use bot_adapters::venue::VenueSnapshot;
use bot_types::Venue;

/// Result of the simplified weighted fair-value computation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FairValue {
    /// Depth-weighted mid price across contributing venues.
    pub p_star: f64,
    /// Sum of weights (total USD depth at top of book across all venues).
    pub total_weight: f64,
    /// Venues that contributed to this computation (depth > 0).
    pub contributing_venues: Vec<Venue>,
    /// True when at least 2 venues contributed. The demo uses this as the
    /// I-FV gate: `healthy == false` means do not emit orders (Step C+).
    pub healthy: bool,
}

/// Compute a depth-weighted mid price from a slice of venue snapshots.
///
/// Weight for each snapshot = `depth_top_usd`. Snapshots with depth ≤ 0.0
/// are excluded (they'd produce a 0-weight contribution and could mask
/// unhealthy venues).
///
/// `healthy == true` when ≥ 2 venues contributed.
pub fn compute_weighted_fair_value(snapshots: &[VenueSnapshot]) -> FairValue {
    let mut weighted_sum = 0.0_f64;
    let mut total_weight = 0.0_f64;
    let mut contributing_venues: Vec<Venue> = Vec::new();

    for snap in snapshots {
        let w = snap.depth_top_usd;
        if w <= 0.0 {
            continue;
        }
        weighted_sum += snap.mid_price * w;
        total_weight += w;
        contributing_venues.push(snap.venue);
    }

    let p_star = if total_weight > 0.0 {
        weighted_sum / total_weight
    } else {
        0.0
    };

    let healthy = contributing_venues.len() >= 2;

    FairValue {
        p_star,
        total_weight,
        contributing_venues,
        healthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_adapters::venue::VenueSnapshot;
    use bot_types::{AnnualizedRate, HourlyRate, Usd};

    fn make_snap(venue: Venue, mid: f64, depth: f64, funding_annual: f64) -> VenueSnapshot {
        VenueSnapshot {
            venue,
            symbol: "BTC".to_string(),
            ts_ms: 1_776_225_863_000,
            mid_price: mid,
            bid_price: mid - 5.0,
            ask_price: mid + 5.0,
            tick_size: 0.1,
            mark_bias_bps: 0.0,
            depth_top_usd: depth,
            depth_curve: vec![
                (1.0, depth),
                (2.0, depth),
                (5.0, depth),
                (10.0, depth),
                (20.0, depth),
            ],
            funding_rate_annual: AnnualizedRate(funding_annual),
            funding_rate_hourly: HourlyRate(funding_annual / (365.0 * 24.0)),
            funding_interval_seconds: 28800,
            next_funding_ts_ms: 1_776_225_863_000 + 28800 * 1000,
            volume_24h_usd: Usd(1_000_000.0),
            open_interest_usd: Usd(500_000.0),
        }
    }

    #[test]
    fn two_equal_depth_venues_midpoint() {
        let snaps = vec![
            make_snap(Venue::Hyperliquid, 100_000.0, 50_000.0, 0.04),
            make_snap(Venue::Lighter, 100_100.0, 50_000.0, 0.03),
        ];
        let fv = compute_weighted_fair_value(&snaps);
        assert!(fv.healthy);
        assert_eq!(fv.contributing_venues.len(), 2);
        // Equal weights → simple average
        assert!((fv.p_star - 100_050.0).abs() < 1e-8);
    }

    #[test]
    fn zero_depth_venue_excluded() {
        let snaps = vec![
            make_snap(Venue::Hyperliquid, 100_000.0, 0.0, 0.04),
            make_snap(Venue::Lighter, 100_100.0, 50_000.0, 0.03),
        ];
        let fv = compute_weighted_fair_value(&snaps);
        assert!(!fv.healthy); // only 1 venue contributed
        assert_eq!(fv.contributing_venues.len(), 1);
        assert!((fv.p_star - 100_100.0).abs() < 1e-8);
    }

    #[test]
    fn empty_snapshots_unhealthy() {
        let fv = compute_weighted_fair_value(&[]);
        assert!(!fv.healthy);
        assert_eq!(fv.p_star, 0.0);
        assert_eq!(fv.total_weight, 0.0);
    }
}
