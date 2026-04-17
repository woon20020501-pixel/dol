//! Venue concentration cap via Herfindahl-Hirschman Index (HHI).
//!
//! Reference: Hirschman, A.O. (1964), "The Paternity of an Index",
//! American Economic Review 54:761-762. Also Rhoades (1993), "The Herfindahl-
//! Hirschman Index", Federal Reserve Bulletin 79(3):188-189.
//!
//! HHI for N venues with exposure fractions s_1, ..., s_N (∑ s_i = 1):
//!
//! ```text
//!     HHI = Σ s_i² ∈ [1/N, 1]
//! ```
//!
//! - HHI = 1     → single venue (maximum concentration).
//! - HHI = 1/N   → perfectly diversified (minimum concentration).
//!
//! Policy (defaults):
//! - `HHI < 0.40` (US DOJ "unconcentrated" threshold) → Pass
//! - `0.40 ≤ HHI < 0.60` → Reduce 0.5 (moderately concentrated)
//! - `HHI ≥ 0.60` → Block (highly concentrated; new entries forbidden)
//!
//! Rationale: DOJ/FTC Horizontal Merger Guidelines use HHI = 0.25 as the
//! moderate-concentration cutoff but we scale the thresholds up because a
//! 4-venue universe has minimum HHI 0.25 already, so <0.40 is materially
//! diversified.

use std::collections::BTreeMap;

use bot_types::Venue;

use super::RiskDecision;

pub const DEFAULT_WARN_HHI: f64 = 0.40;
pub const DEFAULT_BLOCK_HHI: f64 = 0.60;

/// Current USD exposure per venue. Keys are every venue the bot has a
/// non-zero position on, summed across symbols (long + short legs are
/// treated as SEPARATE exposures because each leg consumes margin at a
/// distinct venue).
pub type VenueExposures = BTreeMap<Venue, f64>;

#[derive(Debug, Clone)]
pub struct VenueConcentrationCap {
    warn_hhi: f64,
    block_hhi: f64,
}

impl VenueConcentrationCap {
    pub fn new() -> Self {
        Self::with_thresholds(DEFAULT_WARN_HHI, DEFAULT_BLOCK_HHI)
    }

    pub fn with_thresholds(warn: f64, block: f64) -> Self {
        assert!(0.0 < warn && warn < block && block <= 1.0);
        Self {
            warn_hhi: warn,
            block_hhi: block,
        }
    }

    /// HHI of the given exposure map. Returns 0.0 for empty map.
    pub fn hhi(exposures: &VenueExposures) -> f64 {
        let total: f64 = exposures.values().map(|v| v.max(0.0)).sum();
        if total <= 0.0 {
            return 0.0;
        }
        exposures
            .values()
            .map(|v| {
                let s = v.max(0.0) / total;
                s * s
            })
            .sum()
    }

    /// Evaluate current concentration against the thresholds.
    pub fn check(&self, exposures: &VenueExposures) -> RiskDecision {
        let h = Self::hhi(exposures);
        if h < self.warn_hhi {
            return RiskDecision::Pass;
        }
        if h < self.block_hhi {
            return RiskDecision::Reduce {
                size_multiplier: 0.5,
                reason: format!(
                    "venue HHI {:.3} ∈ [{:.2}, {:.2}) — moderately concentrated",
                    h, self.warn_hhi, self.block_hhi
                ),
            };
        }
        RiskDecision::Block {
            reason: format!(
                "venue HHI {:.3} ≥ {:.2} — highly concentrated",
                h, self.block_hhi
            ),
        }
    }
}

impl Default for VenueConcentrationCap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposures(v: &[(Venue, f64)]) -> VenueExposures {
        v.iter().copied().collect()
    }

    #[test]
    fn empty_is_zero_hhi() {
        let e = VenueExposures::new();
        assert_eq!(VenueConcentrationCap::hhi(&e), 0.0);
    }

    #[test]
    fn single_venue_is_hhi_one() {
        let e = exposures(&[(Venue::Pacifica, 1000.0)]);
        assert!((VenueConcentrationCap::hhi(&e) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn even_4_venues_is_hhi_quarter() {
        let e = exposures(&[
            (Venue::Pacifica, 250.0),
            (Venue::Hyperliquid, 250.0),
            (Venue::Lighter, 250.0),
            (Venue::Backpack, 250.0),
        ]);
        assert!((VenueConcentrationCap::hhi(&e) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn even_2_venues_is_hhi_half() {
        let e = exposures(&[(Venue::Pacifica, 500.0), (Venue::Lighter, 500.0)]);
        assert!((VenueConcentrationCap::hhi(&e) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn diversified_passes() {
        let cap = VenueConcentrationCap::new();
        let e = exposures(&[
            (Venue::Pacifica, 250.0),
            (Venue::Hyperliquid, 250.0),
            (Venue::Lighter, 250.0),
            (Venue::Backpack, 250.0),
        ]);
        assert_eq!(cap.check(&e), RiskDecision::Pass);
    }

    #[test]
    fn two_venue_split_reduces() {
        let cap = VenueConcentrationCap::new();
        let e = exposures(&[(Venue::Pacifica, 500.0), (Venue::Lighter, 500.0)]);
        // HHI=0.5 ∈ [0.40, 0.60) → Reduce
        match cap.check(&e) {
            RiskDecision::Reduce { .. } => {}
            other => panic!("expected Reduce for HHI=0.5, got {:?}", other),
        }
    }

    #[test]
    fn single_venue_blocks() {
        let cap = VenueConcentrationCap::new();
        let e = exposures(&[(Venue::Pacifica, 1000.0)]);
        assert!(matches!(cap.check(&e), RiskDecision::Block { .. }));
    }

    #[test]
    fn negative_or_zero_exposures_handled() {
        let e = exposures(&[(Venue::Pacifica, -100.0), (Venue::Lighter, 0.0)]);
        assert_eq!(VenueConcentrationCap::hhi(&e), 0.0);
    }
}
