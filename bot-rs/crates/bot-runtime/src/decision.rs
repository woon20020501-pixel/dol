//! Cross-venue funding-spread decision rule.
//!
//! Given a vector of per-venue `VenueSnapshot`s for a single symbol, pick
//! the pair `(long_venue, short_venue)` that maximizes
//!
//! ```text
//!     net_spread  =  |funding[short] − funding[long]|  −  round_trip_cost
//! ```
//!
//! where `round_trip_cost` is computed from the cost model in
//! [`bot_math::slippage`] plus per-venue maker/taker fees from
//! `bot_types::Venue::{maker_fee_bps, taker_fee_bps}`. A pair is admitted
//! only if its **net** spread (after cost) clears `min_spread_annual`.
//!
//! Sign convention (`PRINCIPLES.md` §1): the long leg goes to the venue
//! with the LOWER funding rate. If funding is positive (longs pay
//! shorts), being short on the higher-rate venue earns that rate while
//! being long on the lower-rate venue pays less — net income is the
//! rate differential.
//!
//! ## Hysteresis / rebalance policy
//!
//! Once a symbol is holding a pair, `decide` refreshes the notional and
//! the freshly-recomputed spread but **does not propose a different
//! pair**. Rationale: at `--accel-factor 3600` the demo crosses one
//! funding-cycle boundary every real second, and live Pacifica funding
//! rates jitter on ~1 pp timescales. A relative-hysteresis gate becomes
//! ineffective when the held pair's current spread degrades — the
//! threshold shrinks with it and churn restarts. The explicit
//! no-rebalance policy is documented in `docs/v0-punchlist.md` T3-29 as
//! a pre-Tier-1-live cleanup item; production will use
//! `funding_cycle_lock` + `forecast_scoring` + `fsm_controller` +
//! `cvar_guard` to drive rebalance decisions on meaningful signals.
//!
//! ## Iron-law preservation
//!
//! - `PairDecision.symbol` is one `String` shared by both legs (I-SAME).
//! - `PairDecision.{long_venue, short_venue}` are closed-enum `Venue`
//!   values from the 4-DEX whitelist (I-VENUE).
//! - `would_have_executed` is always `true` in demo mode; every decision
//!   is telemetry-only. No adapter is called to submit.

use bot_adapters::venue::VenueSnapshot;
use bot_math::cost::slippage;
use bot_types::{Usd, Venue};
use tracing::info;

/// A "would have executed" pair decision — logged only, never submitted.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PairDecision {
    /// The venue receiving net funding income (long leg — lower funding rate).
    pub long_venue: Venue,
    /// The venue paying the higher funding rate (short leg).
    pub short_venue: Venue,
    /// Symbol (byte-identical on both legs — I-SAME).
    pub symbol: String,
    /// Absolute annualized spread (gross, before round-trip cost).
    pub spread_annual: f64,
    /// Estimated round-trip cost as a dimensionless fraction of notional,
    /// summed over maker+taker fees on both legs + slippage on both legs.
    /// Computed via [`bot_math::cost::slippage`] + per-venue fee constants.
    pub cost_fraction: f64,
    /// Net annualized return after applying the round-trip cost once per
    /// cycle (spread_annual − cost_fraction). Used as the tie-breaker when
    /// multiple pairs clear `min_spread_annual`.
    pub net_annual: f64,
    /// Notional size per leg, in USD. Demo policy: 1 % of portfolio NAV.
    pub notional_usd: f64,
    /// Human-readable explanation logged at INFO. Built **once** for the
    /// winning candidate; never allocated inside the inner enumeration loop.
    pub reason: String,
    /// Always true in the demo — every qualifying spread is logged as
    /// "would execute".
    pub would_have_executed: bool,
}

/// Per-pair notional as a fraction of current NAV (1 % per PRINCIPLES.md §5.1).
const NOTIONAL_FRACTION_OF_NAV: f64 = 0.01;

/// Evaluate a slice of snapshots (same symbol, different venues) and return
/// the best pair decision, or `None` if no pair qualifies.
///
/// - `nav_usd` — caller's current portfolio NAV; drives notional sizing.
/// - `min_spread_annual` — minimum **net** annualized spread (after cost)
///   that a pair must clear to be admitted.
/// - `current_held` — if `Some`, the no-rebalance policy refreshes the
///   held pair's spread from fresh snapshots and returns it verbatim,
///   regardless of whether another pair would score higher.
///
/// Hot-path contract:
/// - Allocates **at most one** `String` (the winning decision's `reason`).
/// - Inner enumeration touches only stack values.
/// - No collection resizing, no `clone`, no `format!` until the winner is
///   chosen.
pub fn decide(
    snapshots: &[VenueSnapshot],
    nav_usd: f64,
    min_spread_annual: f64,
    current_held: Option<&PairDecision>,
) -> Option<PairDecision> {
    // ── No-rebalance policy: if a pair is already held, return it refreshed. ──
    //
    // This branch dominates every tick after the first one. We recompute the
    // held pair's spread from the current snapshot data (so NAV accrual sees
    // the fresh rate each tick) and return. No pair enumeration, no cost
    // model, no `format!`.
    if let Some(held) = current_held {
        let fresh_spread = recompute_spread(snapshots, held.long_venue, held.short_venue)
            .unwrap_or(held.spread_annual);
        let notional = nav_usd * NOTIONAL_FRACTION_OF_NAV;
        return Some(PairDecision {
            long_venue: held.long_venue,
            short_venue: held.short_venue,
            symbol: held.symbol.clone(),
            spread_annual: fresh_spread,
            cost_fraction: held.cost_fraction, // stable across holds
            net_annual: fresh_spread - held.cost_fraction,
            notional_usd: notional,
            reason: format!(
                "hold {}/{} spread={:.2}bps cost={:.2}bps (no-rebalance)",
                held.long_venue.as_str(),
                held.short_venue.as_str(),
                fresh_spread * 10_000.0,
                held.cost_fraction * 10_000.0,
            ),
            would_have_executed: true,
        });
    }

    // ── Full pair enumeration (first tick or post-close) ──
    if snapshots.len() < 2 {
        return None;
    }

    let notional = nav_usd * NOTIONAL_FRACTION_OF_NAV;
    let n_per_leg = Usd(notional);

    // Winner tracking — stack only, no allocation until the final PairDecision.
    let mut best_idx: Option<(usize, usize)> = None;
    let mut best_net: f64 = f64::NEG_INFINITY;
    let mut best_spread: f64 = 0.0;
    let mut best_cost: f64 = 0.0;

    for i in 0..snapshots.len() {
        for j in (i + 1)..snapshots.len() {
            let a = &snapshots[i];
            let b = &snapshots[j];

            let rate_a = a.funding_rate_annual.0;
            let rate_b = b.funding_rate_annual.0;

            // Long leg = lower funding rate (I-LOCK sign convention).
            let (long_snap, short_snap, spread) = if rate_a <= rate_b {
                (a, b, rate_b - rate_a)
            } else {
                (b, a, rate_a - rate_b)
            };

            // Reject negative depth / degenerate cases early.
            if long_snap.depth_top_usd <= 0.0 || short_snap.depth_top_usd <= 0.0 {
                continue;
            }

            // ── Cost model (bot-math) ──
            //
            // Per-leg slippage via sqrt-impact. Fees: pivot leg pays
            // maker+taker (Model C round trip), counter leg pays 2× taker.
            // By convention we treat the LONG leg as the pivot (maker),
            // because the decision opens with a limit order on the lower-
            // funding side and hedges with a taker order on the higher-
            // funding side. The symmetric cost is charged once per cycle.
            let slip_long = slippage(
                n_per_leg,
                long_snap.open_interest_usd,
                long_snap.volume_24h_usd,
            )
            .0;
            let slip_short = slippage(
                n_per_leg,
                short_snap.open_interest_usd,
                short_snap.volume_24h_usd,
            )
            .0;

            let pivot_maker_bps = long_snap.venue.maker_fee_bps();
            let pivot_taker_bps = long_snap.venue.taker_fee_bps();
            let counter_taker_bps = short_snap.venue.taker_fee_bps();

            // c^C = (φ_m^{v_p} + φ_t^{v_p}) + 2·φ_t^{v_c} + σ(n,Π_p) + 2·σ(n,Π_c)
            // Fees are per-dollar fractions (bps × 1e-4). Slippage is already
            // a dimensionless fraction from `bot_math::slippage`.
            let fee_cost = (pivot_maker_bps + pivot_taker_bps + 2.0 * counter_taker_bps) * 1e-4;
            let slip_cost = slip_long + 2.0 * slip_short;
            let cost_fraction = fee_cost + slip_cost;

            // Net annualized return after one-time round-trip cost.
            // Interpretation: `cost_fraction` is paid once at open; the
            // spread accrues continuously. On a one-year hold the break-
            // even condition is `spread ≥ cost`. At shorter holds the
            // break-even target shifts higher — caller's `funding_cycle_lock`
            // enforces the hold window.
            let net_annual = spread - cost_fraction;

            if net_annual < min_spread_annual {
                continue;
            }

            if net_annual > best_net {
                best_idx = Some(if rate_a <= rate_b { (i, j) } else { (j, i) });
                best_net = net_annual;
                best_spread = spread;
                best_cost = cost_fraction;
            }
        }
    }

    let (long_idx, short_idx) = best_idx?;
    let long_snap = &snapshots[long_idx];
    let short_snap = &snapshots[short_idx];

    // Build the winning decision's Reason string exactly once.
    let reason = format!(
        "long {}({:.4}% pa) short {}({:.4}% pa) spread={:.2}bps cost={:.2}bps net={:.2}bps notional=${:.0}",
        long_snap.venue.as_str(),
        long_snap.funding_rate_annual.0 * 100.0,
        short_snap.venue.as_str(),
        short_snap.funding_rate_annual.0 * 100.0,
        best_spread * 10_000.0,
        best_cost * 10_000.0,
        best_net * 10_000.0,
        notional,
    );

    let decision = PairDecision {
        long_venue: long_snap.venue,
        short_venue: short_snap.venue,
        symbol: long_snap.symbol.clone(),
        spread_annual: best_spread,
        cost_fraction: best_cost,
        net_annual: best_net,
        notional_usd: notional,
        reason,
        would_have_executed: true,
    };

    info!(
        symbol = %decision.symbol,
        long = ?decision.long_venue,
        short = ?decision.short_venue,
        spread_bps = best_spread * 10_000.0,
        cost_bps = best_cost * 10_000.0,
        net_bps = best_net * 10_000.0,
        notional = notional,
        "would_have_executed"
    );

    Some(decision)
}

/// Recompute the absolute annualized spread between two venues from the
/// current tick's snapshots. Returns `None` if either venue is missing.
#[inline]
fn recompute_spread(snapshots: &[VenueSnapshot], long_v: Venue, short_v: Venue) -> Option<f64> {
    let long_rate = snapshots
        .iter()
        .find(|s| s.venue == long_v)
        .map(|s| s.funding_rate_annual.0)?;
    let short_rate = snapshots
        .iter()
        .find(|s| s.venue == short_v)
        .map(|s| s.funding_rate_annual.0)?;
    Some((short_rate - long_rate).abs())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bot_adapters::venue::VenueSnapshot;
    use bot_types::{AnnualizedRate, HourlyRate, Usd};

    fn make_snap(venue: Venue, funding_annual: f64) -> VenueSnapshot {
        VenueSnapshot {
            venue,
            symbol: "BTC".to_string(),
            ts_ms: 1_776_225_863_000,
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
            funding_interval_seconds: 28800,
            next_funding_ts_ms: 1_776_225_863_000 + 28_800_000,
            volume_24h_usd: Usd(1_000_000_000.0),
            open_interest_usd: Usd(500_000_000.0),
        }
    }

    /// The picked pair's `long_venue` has strictly lower funding than
    /// `short_venue`. Tests the iron-law §1 sign convention.
    #[test]
    fn picks_long_venue_with_lower_funding() {
        let snaps = vec![
            make_snap(Venue::Lighter, 0.03),
            make_snap(Venue::Backpack, 0.06),
            make_snap(Venue::Hyperliquid, 0.04),
        ];
        let d = decide(&snaps, 10_000.0, 0.0002, None).unwrap();
        assert_eq!(d.long_venue, Venue::Lighter);
        assert_eq!(d.short_venue, Venue::Backpack);
        assert!((d.spread_annual - 0.03).abs() < 1e-10);
        assert!(d.cost_fraction > 0.0, "cost must be positive");
        assert!(d.net_annual < d.spread_annual, "net < gross after cost");
        assert!((d.notional_usd - 100.0).abs() < 1e-8); // 1% of $10k
    }

    /// Reject when the net spread (after cost) falls below the gate.
    #[test]
    fn rejects_when_cost_dominates_gross() {
        let snaps = vec![
            make_snap(Venue::Lighter, 0.03),
            make_snap(Venue::Hyperliquid, 0.03001), // spread = 0.01 bps
        ];
        assert!(
            decide(&snaps, 10_000.0, 0.0002, None).is_none(),
            "0.01 bps spread cannot clear any positive cost gate"
        );
    }

    #[test]
    fn no_decision_single_venue() {
        let snaps = vec![make_snap(Venue::Lighter, 0.05)];
        assert!(decide(&snaps, 10_000.0, 0.0002, None).is_none());
    }

    #[test]
    fn no_rebalance_holds_even_when_alt_is_better() {
        let held = PairDecision {
            long_venue: Venue::Lighter,
            short_venue: Venue::Backpack,
            symbol: "BTC".to_string(),
            spread_annual: 0.03,
            cost_fraction: 0.0015,
            net_annual: 0.0285,
            notional_usd: 100.0,
            reason: "prior".to_string(),
            would_have_executed: true,
        };
        let snaps = vec![
            make_snap(Venue::Lighter, 0.03),
            make_snap(Venue::Backpack, 0.06),
            make_snap(Venue::Hyperliquid, 0.12), // alt 9% — much better
        ];
        let d = decide(&snaps, 10_000.0, 0.0002, Some(&held)).unwrap();
        assert_eq!(d.long_venue, Venue::Lighter);
        assert_eq!(d.short_venue, Venue::Backpack);
    }

    #[test]
    fn held_spread_refreshes_from_current_snapshots() {
        let held = PairDecision {
            long_venue: Venue::Lighter,
            short_venue: Venue::Backpack,
            symbol: "BTC".to_string(),
            spread_annual: 0.03, // opened at 3 %
            cost_fraction: 0.0015,
            net_annual: 0.0285,
            notional_usd: 100.0,
            reason: "prior".to_string(),
            would_have_executed: true,
        };
        let snaps = vec![
            make_snap(Venue::Lighter, 0.04),
            make_snap(Venue::Backpack, 0.10), // spread widened to 6 %
            make_snap(Venue::Hyperliquid, 0.05),
        ];
        let d = decide(&snaps, 10_000.0, 0.0002, Some(&held)).unwrap();
        assert_eq!(d.long_venue, Venue::Lighter);
        assert_eq!(d.short_venue, Venue::Backpack);
        assert!((d.spread_annual - 0.06).abs() < 1e-10);
        assert!((d.net_annual - (0.06 - held.cost_fraction)).abs() < 1e-10);
    }

    /// Pair with zero-depth counter venue is skipped even if spread is wide.
    #[test]
    fn skips_zero_depth_venue() {
        let mut thin = make_snap(Venue::Backpack, 0.20);
        thin.depth_top_usd = 0.0;
        let snaps = vec![make_snap(Venue::Lighter, 0.03), thin];
        assert!(
            decide(&snaps, 10_000.0, 0.0002, None).is_none(),
            "zero-depth short leg must be rejected regardless of headline spread"
        );
    }

    /// Cost is roughly symmetric across venue permutations of the same pair
    /// (same fee schedules → same slippage inputs → deterministic cost).
    #[test]
    fn cost_is_deterministic_for_fixed_inputs() {
        let snaps = vec![
            make_snap(Venue::Lighter, 0.03),
            make_snap(Venue::Backpack, 0.06),
        ];
        let d1 = decide(&snaps, 10_000.0, 0.0002, None).unwrap();
        let d2 = decide(&snaps, 10_000.0, 0.0002, None).unwrap();
        assert_eq!(d1.cost_fraction, d2.cost_fraction);
        assert_eq!(d1.spread_annual, d2.spread_annual);
        assert_eq!(d1.net_annual, d2.net_annual);
    }
}
