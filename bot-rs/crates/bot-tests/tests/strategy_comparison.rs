//! Comparative benchmark: naive funding-max vs cost-aware decision.
//!
//! Runs the same 500-tick synthetic spread series through two strategies:
//!
//! 1. **Naive** — picks the pair with the largest absolute spread every
//!    tick; no cost filter, no hysteresis, no regime awareness.
//! 2. **Smart** — `decision::decide` with round-trip cost admission +
//!    no-rebalance policy from `decision.rs`.
//!
//! Asserts the smart strategy:
//!   a) produces strictly fewer rebalances (no-rebalance policy), AND
//!   b) beats the naive strategy in final NAV net-of-cost.
//!
//! This is a reproducible deterministic benchmark — no RNG, no network,
//! no file I/O. Fixture series is synthesized from a 3-mode funding schedule
//! that is known to break naive max-spread:
//! - phase A (ticks 0..166):   pair_1 highest, pair_2 close
//! - phase B (ticks 167..333): pair_2 overtakes by 1 bps — NAIVE rebalances,
//!   SMART holds (cost gate rejects 1 bps gain).
//! - phase C (ticks 334..500): pair_1 back on top — NAIVE rebalances again,
//!   SMART held through, saves 2× round-trip cost.

use bot_adapters::venue::VenueSnapshot;
use bot_runtime::decision::{self, PairDecision};
use bot_runtime::nav::NavTracker;
use bot_types::{AnnualizedRate, HourlyRate, Usd, Venue};

const TICK_COUNT: usize = 500;
const DT_SECONDS: f64 = 3600.0; // 1-hour ticks
const STARTING_NAV: f64 = 10_000.0;

fn make_snap(venue: Venue, symbol: &str, funding_annual: f64) -> VenueSnapshot {
    VenueSnapshot {
        venue,
        symbol: symbol.to_string(),
        ts_ms: 0,
        mid_price: 100_000.0,
        bid_price: 99_995.0,
        ask_price: 100_005.0,
        tick_size: 0.1,
        mark_bias_bps: 0.0,
        depth_top_usd: 50_000.0,
        depth_curve: vec![
            (1.0, 50_000.0),
            (2.0, 50_000.0),
            (5.0, 50_000.0),
            (10.0, 50_000.0),
            (20.0, 50_000.0),
        ],
        funding_rate_annual: AnnualizedRate(funding_annual),
        funding_rate_hourly: HourlyRate(funding_annual / (365.0 * 24.0)),
        funding_interval_seconds: 3600,
        next_funding_ts_ms: 0,
        volume_24h_usd: Usd(1_000_000_000.0),
        open_interest_usd: Usd(500_000_000.0),
    }
}

/// Build the funding-rate schedule for tick `t`.
/// Returns 4 snapshots; phase determines which pair is "best".
fn snapshots_for_tick(t: usize) -> Vec<VenueSnapshot> {
    let phase = match t {
        0..=166 => 0,
        167..=333 => 1,
        _ => 2,
    };
    // Baseline 3% / 5% spread between Lighter and Backpack (the "pair_1" edge).
    // In phase 1, Hyperliquid overtakes Backpack by 1 bps (near-zero net value
    // gain but naive will chase it).
    match phase {
        0 => vec![
            make_snap(Venue::Lighter, "BTC", 0.03),
            make_snap(Venue::Backpack, "BTC", 0.08), // spread 5%
            make_snap(Venue::Hyperliquid, "BTC", 0.05),
            make_snap(Venue::Pacifica, "BTC", 0.04),
        ],
        1 => vec![
            make_snap(Venue::Lighter, "BTC", 0.03),
            make_snap(Venue::Backpack, "BTC", 0.08001), // still leads by a whisker
            make_snap(Venue::Hyperliquid, "BTC", 0.08002), // overtakes by 1 bps of annual spread
            make_snap(Venue::Pacifica, "BTC", 0.04),
        ],
        _ => vec![
            make_snap(Venue::Lighter, "BTC", 0.03),
            make_snap(Venue::Backpack, "BTC", 0.08), // leads again
            make_snap(Venue::Hyperliquid, "BTC", 0.05),
            make_snap(Venue::Pacifica, "BTC", 0.04),
        ],
    }
}

/// Naive strategy: always open the max-spread pair. No cost filter.
fn naive_decide(snaps: &[VenueSnapshot], nav_usd: f64) -> Option<PairDecision> {
    if snaps.len() < 2 {
        return None;
    }
    let notional = nav_usd * 0.01;
    // Find pair with max |spread|.
    let mut best: Option<(usize, usize, f64)> = None;
    for i in 0..snaps.len() {
        for j in (i + 1)..snaps.len() {
            let (a, b) = (&snaps[i], &snaps[j]);
            let spread = (a.funding_rate_annual.0 - b.funding_rate_annual.0).abs();
            match best {
                None => best = Some((i, j, spread)),
                Some((_, _, cur)) if spread > cur => best = Some((i, j, spread)),
                _ => {}
            }
        }
    }
    let (i, j, spread) = best?;
    let (a, b) = (&snaps[i], &snaps[j]);
    let (long_snap, short_snap) = if a.funding_rate_annual.0 < b.funding_rate_annual.0 {
        (a, b)
    } else {
        (b, a)
    };
    Some(PairDecision {
        long_venue: long_snap.venue,
        short_venue: short_snap.venue,
        symbol: "BTC".to_string(),
        spread_annual: spread,
        cost_fraction: 0.0015, // same cost_fraction used by smart (apples-to-apples)
        net_annual: spread - 0.0015,
        notional_usd: notional,
        reason: "naive".to_string(),
        would_have_executed: true,
    })
}

#[test]
fn smart_beats_naive_after_500_ticks() {
    let mut naive_nav = NavTracker::new(STARTING_NAV);
    let mut smart_nav = NavTracker::new(STARTING_NAV);

    let mut naive_rebalances = 0;
    let mut smart_rebalances = 0;
    let mut prev_naive: Option<(Venue, Venue)> = None;
    let mut prev_smart: Option<(Venue, Venue)> = None;
    let mut smart_held: Option<PairDecision> = None;

    for t in 0..TICK_COUNT {
        let snaps = snapshots_for_tick(t);

        // Naive
        let naive_d = naive_decide(&snaps, naive_nav.nav_usd);
        if let Some(ref d) = naive_d {
            let cur = (d.long_venue, d.short_venue);
            if let Some(prev) = prev_naive {
                if prev != cur {
                    naive_rebalances += 1;
                }
            }
            prev_naive = Some(cur);
        }
        naive_nav.accrue((t as i64) * 3_600_000, naive_d.as_ref(), DT_SECONDS);

        // Smart
        let smart_d = decision::decide(&snaps, smart_nav.nav_usd, 0.0002, smart_held.as_ref());
        if let Some(ref d) = smart_d {
            let cur = (d.long_venue, d.short_venue);
            if let Some(prev) = prev_smart {
                if prev != cur {
                    smart_rebalances += 1;
                }
            }
            prev_smart = Some(cur);
        }
        smart_nav.accrue((t as i64) * 3_600_000, smart_d.as_ref(), DT_SECONDS);
        smart_held = smart_d;
    }

    println!(
        "naive: NAV={:.4} rebalances={} fees_paid={:.4}",
        naive_nav.nav_usd, naive_rebalances, naive_nav.fees_paid_usd
    );
    println!(
        "smart: NAV={:.4} rebalances={} fees_paid={:.4}",
        smart_nav.nav_usd, smart_rebalances, smart_nav.fees_paid_usd
    );

    // Invariant 1: smart rebalances strictly less often (no-rebalance policy).
    assert!(
        smart_rebalances < naive_rebalances,
        "smart rebalances ({}) must be fewer than naive ({})",
        smart_rebalances,
        naive_rebalances
    );

    // Invariant 2: smart pays strictly fewer fees (fewer entries).
    assert!(
        smart_nav.fees_paid_usd < naive_nav.fees_paid_usd,
        "smart fees ({:.4}) must be less than naive fees ({:.4})",
        smart_nav.fees_paid_usd,
        naive_nav.fees_paid_usd
    );

    // Invariant 3: smart final NAV ≥ naive final NAV (net of all costs).
    assert!(
        smart_nav.nav_usd >= naive_nav.nav_usd,
        "smart NAV ({:.4}) must ≥ naive NAV ({:.4})",
        smart_nav.nav_usd,
        naive_nav.nav_usd
    );
}
