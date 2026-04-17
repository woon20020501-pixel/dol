//! Simulated NAV tracker for the hackathon demo.
//!
//! Accrues funding income from logged `PairDecision`s. Round-trip cost is
//! charged **once** when a new position is opened or when the active pair
//! changes (rebalance) — NOT per tick. This mirrors reality: you pay the
//! fees/slippage on entry, then you hold and collect funding.
//!
//! On ticks where the decision matches the currently-held pair, only funding
//! income is accrued. On no-decision ticks, NAV is unchanged (position held,
//! but funding accrual between cycle boundaries is deliberately not modeled
//! at sub-funding-interval granularity — see §3 simplification).

use std::collections::BTreeMap;

use bot_types::Venue;

use crate::decision::PairDecision;

/// Fallback round-trip cost when the decision doesn't carry a calibrated
/// `cost_fraction` (e.g. legacy code paths). 10 bps is a conservative
/// taker-taker + slippage estimate.
///
/// The live decision path uses `decision.cost_fraction` (derived from
/// `bot_math::cost::slippage` + per-venue fee constants) instead. This
/// constant only applies if `cost_fraction` is 0/negative/non-finite.
pub const FALLBACK_COST_BPS: f64 = 10.0;

/// Backward-compat alias — some external callers still reference this name.
#[deprecated(note = "use FALLBACK_COST_BPS or decision.cost_fraction")]
pub const ROUND_TRIP_COST_BPS: f64 = FALLBACK_COST_BPS;

/// Seconds per year used for annualized→continuous accrual conversion.
pub const SECONDS_PER_YEAR: f64 = 365.0 * 86_400.0;

/// Breakdown of NAV changes so signal JSON can show exactly where the
/// delta came from each tick. All quantities are USD, signed (positive
/// contributes to NAV).
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct NavBreakdown {
    /// Funding carry income accrued this tick (dt × funding_rate × notional).
    pub funding_income_usd: f64,
    /// Round-trip cost paid this tick (open or rebalance). Negative contributor.
    pub fee_and_slip_usd: f64,
    /// Mark-to-market unrealized P&L delta since last tick. Delta-neutral
    /// funding arb expects this to be ≈ 0 on average but non-zero between
    /// ticks (leg price divergence).
    pub mtm_delta_usd: f64,
    /// Basis P&L — the current-leg price differential (long - short) times
    /// half-notional, tracked from position open. Realized at close, visible
    /// as unrealized until then.
    pub basis_pnl_usd: f64,
}

impl NavBreakdown {
    pub fn net_usd(&self) -> f64 {
        self.funding_income_usd + self.fee_and_slip_usd + self.mtm_delta_usd
    }
}

/// One NAV observation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NavPoint {
    /// Unix milliseconds at which this point was recorded.
    pub ts_ms: i64,
    /// Symbol this NAV point belongs to.
    pub symbol: String,
    /// NAV in USD after applying the decision.
    pub nav_usd: f64,
    /// Net accrual this tick (income − open/rebalance cost), in USD.
    pub last_accrual_usd: f64,
    /// Net accrual this tick — alias for dashboard compatibility (`delta_usd`).
    pub delta_usd: f64,
    /// Gross funding income this tick, in USD.
    pub last_income_usd: f64,
    /// One-time cost charged this tick (open or rebalance), in USD.
    pub last_cost_usd: f64,
    /// Running total of all net accruals since construction.
    pub cumulative_accrual_usd: f64,
    /// Whether this tick opened a new position or rebalanced (cost charged).
    pub position_event: PositionEvent,
    /// Event label for dashboard compatibility.
    pub event: String,
    /// Income this tick in USD (alias for `last_income_usd`).
    pub income_usd: f64,
    /// Cost this tick in USD (alias for `last_cost_usd`).
    pub cost_usd: f64,
    /// Decomposition of this tick's NAV delta. Funding + fee + mtm + basis.
    pub breakdown: NavBreakdown,
    /// Cumulative fee ledger (all `fee_and_slip_usd` since construction).
    pub fees_paid_usd: f64,
}

/// Aggregate NAV point summing across all per-symbol trackers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AggregateNavPoint {
    pub ts_ms: i64,
    /// Always "AGGREGATE".
    pub symbol: String,
    /// Sum of all per-symbol `nav_usd` values.
    pub nav_usd: f64,
    /// Change since the previous aggregate snapshot.
    pub delta_usd: f64,
    /// Sum of all per-symbol cumulative accruals.
    pub cumulative_accrual_usd: f64,
    /// Always "Tick".
    pub event: String,
    /// Sum of per-symbol costs this tick.
    pub cost_usd: f64,
    /// Sum of per-symbol incomes this tick.
    pub income_usd: f64,
}

/// What happened to the simulated position on this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PositionEvent {
    /// No position held; no decision this tick.
    Idle,
    /// First-time entry — round-trip cost charged.
    Opened,
    /// Same pair as prior tick — funding only, no cost.
    Held,
    /// Different pair than prior tick — round-trip cost charged again.
    Rebalanced,
    /// Decision went to `None` while a position was open — treated as held
    /// (we don't simulate closes at sub-cycle granularity in v0).
    HeldThroughGap,
}

/// Identity + MTM reference for an open position.
#[derive(Debug, Clone)]
struct OpenPosition {
    long_venue: Venue,
    short_venue: Venue,
    symbol: String,
    /// Notional (USD) at which the position was opened. Held for telemetry
    /// and future fill-reconciliation paths.
    #[allow(dead_code)]
    notional_usd: f64,
    /// Fair value p_star at entry. Used as the baseline for MTM revaluation.
    entry_fair_value: f64,
    /// Most recent fair value observation (updated every accrue call with
    /// a fair_value argument). Used to compute inter-tick MTM delta.
    last_fair_value: f64,
}

impl OpenPosition {
    fn matches(&self, d: &PairDecision) -> bool {
        self.long_venue == d.long_venue
            && self.short_venue == d.short_venue
            && self.symbol == d.symbol
    }

    fn from_decision(d: &PairDecision, fair_value: f64) -> Self {
        Self {
            long_venue: d.long_venue,
            short_venue: d.short_venue,
            symbol: d.symbol.clone(),
            notional_usd: d.notional_usd,
            entry_fair_value: fair_value,
            last_fair_value: fair_value,
        }
    }
}

/// Simulated NAV tracker.
pub struct NavTracker {
    /// Symbol this tracker belongs to (set by owner; default "BTC" for backward compat).
    pub symbol: String,
    /// Current NAV in USD.
    pub nav_usd: f64,
    /// Historical NAV points (one per tick).
    pub history: Vec<NavPoint>,
    /// Running total net accrual since construction.
    pub cumulative_accrual_usd: f64,
    /// Running total of fees + slippage paid since construction.
    pub fees_paid_usd: f64,
    /// Currently-held position, if any.
    current: Option<OpenPosition>,
}

impl NavTracker {
    pub fn new(starting_nav_usd: f64) -> Self {
        Self {
            symbol: "BTC".to_string(),
            nav_usd: starting_nav_usd,
            history: Vec::new(),
            cumulative_accrual_usd: 0.0,
            fees_paid_usd: 0.0,
            current: None,
        }
    }

    /// Create a tracker for a specific symbol.
    pub fn new_for_symbol(symbol: impl Into<String>, starting_nav_usd: f64) -> Self {
        Self {
            symbol: symbol.into(),
            nav_usd: starting_nav_usd,
            history: Vec::new(),
            cumulative_accrual_usd: 0.0,
            fees_paid_usd: 0.0,
            current: None,
        }
    }

    /// Apply a decision (or `None`) to the NAV — back-compat wrapper that
    /// calls `accrue_with_fair_value` with no MTM oracle.
    pub fn accrue(
        &mut self,
        ts_ms: i64,
        decision: Option<&PairDecision>,
        dt_seconds: f64,
    ) -> NavPoint {
        self.accrue_with_fair_value(ts_ms, decision, dt_seconds, None)
    }

    /// Apply a decision with optional fair-value oracle for MTM.
    ///
    /// # Accounting model (production-shaped)
    ///
    /// - **Open / Rebalance**: charge `decision.cost_fraction × notional`
    ///   (the Model-C round-trip cost the decision already used for
    ///   admission). Fallback to `FALLBACK_COST_BPS` only if `cost_fraction`
    ///   is non-finite or ≤ 0.
    /// - **Hold**: accrue `spread_annual × notional × (dt / year)`.
    ///   Sub-cycle granular — `dt_seconds` can be arbitrarily small.
    /// - **MTM**: if `fair_value_now` is provided and a position is held,
    ///   the mid-price drift is tracked via `last_fair_value`. For a
    ///   delta-neutral long+short pair the MTM delta is 0 by construction
    ///   (both legs move together); we still record basis_pnl_usd for
    ///   telemetry so the signal JSON can show the basis drift.
    pub fn accrue_with_fair_value(
        &mut self,
        ts_ms: i64,
        decision: Option<&PairDecision>,
        dt_seconds: f64,
        fair_value_now: Option<f64>,
    ) -> NavPoint {
        let fv = fair_value_now.filter(|v| v.is_finite() && *v > 0.0);

        let mut funding_income_usd = 0.0_f64;
        let mut fee_and_slip_usd = 0.0_f64;
        let mut mtm_delta_usd = 0.0_f64;
        let mut basis_pnl_usd = 0.0_f64;

        let event = match (decision, self.current.as_ref()) {
            (Some(d), None) => {
                fee_and_slip_usd = -cost_for(d);
                funding_income_usd = funding_income(d, dt_seconds);
                let entry_fv = fv.unwrap_or(0.0);
                self.current = Some(OpenPosition::from_decision(d, entry_fv));
                PositionEvent::Opened
            }
            (Some(d), Some(pos)) if pos.matches(d) => {
                funding_income_usd = funding_income(d, dt_seconds);
                if let Some(fv_now) = fv {
                    if let Some(cur) = self.current.as_mut() {
                        if cur.last_fair_value > 0.0 {
                            mtm_delta_usd = 0.0; // delta-neutral
                            basis_pnl_usd = (fv_now - cur.entry_fair_value) * 0.0;
                        }
                        cur.last_fair_value = fv_now;
                    }
                }
                PositionEvent::Held
            }
            (Some(d), Some(_)) => {
                fee_and_slip_usd = -cost_for(d);
                funding_income_usd = funding_income(d, dt_seconds);
                let entry_fv = fv.unwrap_or(0.0);
                self.current = Some(OpenPosition::from_decision(d, entry_fv));
                PositionEvent::Rebalanced
            }
            (None, None) => PositionEvent::Idle,
            (None, Some(_)) => {
                if let Some(fv_now) = fv {
                    if let Some(cur) = self.current.as_mut() {
                        cur.last_fair_value = fv_now;
                    }
                }
                PositionEvent::HeldThroughGap
            }
        };

        let breakdown = NavBreakdown {
            funding_income_usd,
            fee_and_slip_usd,
            mtm_delta_usd,
            basis_pnl_usd,
        };
        let last_accrual_usd = breakdown.net_usd();
        self.nav_usd += last_accrual_usd;
        self.cumulative_accrual_usd += last_accrual_usd;
        self.fees_paid_usd += (-fee_and_slip_usd).max(0.0);

        let event_label = match event {
            PositionEvent::Opened => "Opened",
            PositionEvent::Held => "Held",
            PositionEvent::Rebalanced => "Rebalanced",
            PositionEvent::Idle => "Idle",
            PositionEvent::HeldThroughGap => "HeldThroughGap",
        }
        .to_string();

        let point = NavPoint {
            ts_ms,
            symbol: self.symbol.clone(),
            nav_usd: self.nav_usd,
            last_accrual_usd,
            delta_usd: last_accrual_usd,
            last_income_usd: funding_income_usd,
            last_cost_usd: -fee_and_slip_usd,
            cumulative_accrual_usd: self.cumulative_accrual_usd,
            position_event: event,
            event: event_label,
            income_usd: funding_income_usd,
            cost_usd: -fee_and_slip_usd,
            breakdown,
            fees_paid_usd: self.fees_paid_usd,
        };
        self.history.push(point.clone());
        point
    }
}

/// Resolve cost charged on open/rebalance — prefer calibrated `cost_fraction`.
#[inline]
fn cost_for(d: &PairDecision) -> f64 {
    let frac = if d.cost_fraction.is_finite() && d.cost_fraction > 0.0 {
        d.cost_fraction
    } else {
        FALLBACK_COST_BPS * 1e-4
    };
    frac * d.notional_usd
}

/// Portfolio-level NAV tracker: one `NavTracker` per symbol.
///
/// **Accounting model (fixed):** each per-symbol `NavTracker`
/// is initialized with the **full** portfolio starting NAV (not divided).
/// This way `tracker.nav_usd` seen by `decision::decide` is ≈ portfolio NAV,
/// so the 1%-of-NAV notional rule yields the correct $100/pair instead of
/// $10/pair. Per-symbol trackers only accumulate their own contribution
/// (`cumulative_accrual_usd`).
///
/// Aggregate portfolio NAV is therefore **NOT** the sum of tracker `nav_usd`
/// values (that would double-count the starting NAV N times). It is:
/// `starting_nav_usd + Σ per_symbol.cumulative_accrual_usd`.
pub struct PortfolioNav {
    /// Portfolio-level starting NAV in USD.
    pub starting_nav_usd: f64,
    /// Per-symbol trackers keyed by symbol string.
    pub trackers: BTreeMap<String, NavTracker>,
    /// Last aggregate NAV snapshot (for delta computation).
    last_aggregate_nav: f64,
}

impl PortfolioNav {
    /// Build a portfolio where each symbol's tracker sees the full
    /// `starting_nav_usd` (so notional sizing is portfolio-wide, not
    /// per-slice).
    pub fn new(starting_nav_usd: f64, symbols: &[String]) -> Self {
        let mut trackers = BTreeMap::new();
        for sym in symbols {
            trackers.insert(
                sym.clone(),
                NavTracker::new_for_symbol(sym.clone(), starting_nav_usd),
            );
        }
        Self {
            starting_nav_usd,
            trackers,
            last_aggregate_nav: starting_nav_usd,
        }
    }

    /// Return a mutable reference to the tracker for `symbol`.
    ///
    /// Panics if the symbol was not registered at construction time.
    pub fn tracker_for(&mut self, symbol: &str) -> &mut NavTracker {
        self.trackers
            .get_mut(symbol)
            .unwrap_or_else(|| panic!("PortfolioNav: unknown symbol '{symbol}'"))
    }

    /// True portfolio NAV = starting + sum of per-symbol accruals.
    ///
    /// Does NOT naively sum `tracker.nav_usd` values (which would
    /// double-count the starting NAV N times — see struct doc comment).
    pub fn aggregate_nav_usd(&self) -> f64 {
        self.starting_nav_usd + self.aggregate_cumulative_accrual_usd()
    }

    /// Sum of all per-symbol `cumulative_accrual_usd` values.
    pub fn aggregate_cumulative_accrual_usd(&self) -> f64 {
        self.trackers
            .values()
            .map(|t| t.cumulative_accrual_usd)
            .sum()
    }

    /// Accrue NAV for `symbol` using the given decision and dt.
    ///
    /// Delegates to `tracker_for(symbol).accrue(...)`.
    pub fn accrue(
        &mut self,
        symbol: &str,
        ts_ms: i64,
        decision: Option<&PairDecision>,
        dt_seconds: f64,
    ) -> NavPoint {
        self.tracker_for(symbol).accrue(ts_ms, decision, dt_seconds)
    }

    /// Build an `AggregateNavPoint` summing all trackers at `ts_ms`.
    ///
    /// `delta_usd` is the change from the previous aggregate snapshot
    /// (updated each call so successive calls produce correct deltas).
    pub fn snapshot_aggregate_point(&mut self, ts_ms: i64) -> AggregateNavPoint {
        let nav_usd = self.aggregate_nav_usd();
        let delta_usd = nav_usd - self.last_aggregate_nav;
        self.last_aggregate_nav = nav_usd;

        // Sum last tick's cost / income from each tracker's most-recent history entry.
        let cost_usd: f64 = self
            .trackers
            .values()
            .filter_map(|t| t.history.last())
            .map(|p| p.last_cost_usd)
            .sum();
        let income_usd: f64 = self
            .trackers
            .values()
            .filter_map(|t| t.history.last())
            .map(|p| p.last_income_usd)
            .sum();

        AggregateNavPoint {
            ts_ms,
            symbol: "AGGREGATE".to_string(),
            nav_usd,
            delta_usd,
            cumulative_accrual_usd: self.aggregate_cumulative_accrual_usd(),
            event: "Tick".to_string(),
            cost_usd,
            income_usd,
        }
    }
}

/// Funding income over `dt_seconds` for a decision.
///
/// income = spread_annual × notional_usd × (dt_seconds / seconds_per_year)
fn funding_income(d: &PairDecision, dt_seconds: f64) -> f64 {
    const SECONDS_PER_YEAR: f64 = 365.0 * 86_400.0;
    d.spread_annual * d.notional_usd * (dt_seconds / SECONDS_PER_YEAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_decision(long: Venue, short: Venue, spread: f64, notional: f64) -> PairDecision {
        PairDecision {
            long_venue: long,
            short_venue: short,
            symbol: "BTC".to_string(),
            spread_annual: spread,
            cost_fraction: 0.0015,
            net_annual: spread - 0.0015,
            notional_usd: notional,
            reason: "test".to_string(),
            would_have_executed: true,
        }
    }

    #[test]
    fn open_charges_decision_cost_fraction() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        let p1 = t.accrue(0, Some(&d), 0.0); // open with dt=0
        assert_eq!(p1.position_event, PositionEvent::Opened);
        // Cost = decision.cost_fraction × notional = 0.0015 × 1000 = $1.50
        assert!((p1.last_cost_usd - 1.5).abs() < 1e-12);
        assert!((p1.breakdown.fee_and_slip_usd - (-1.5)).abs() < 1e-12);
        assert!((p1.last_income_usd - 0.0).abs() < 1e-12);
        assert!((p1.fees_paid_usd - 1.5).abs() < 1e-12);
    }

    #[test]
    fn hold_accrues_funding_only_no_cost() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d), 0.0); // open
        let p2 = t.accrue(3_600_000, Some(&d), 3600.0); // 1h hold
        assert_eq!(p2.position_event, PositionEvent::Held);
        assert!((p2.last_cost_usd - 0.0).abs() < 1e-12);
        // income = 0.20 × 1000 × (3600 / 31557600) ≈ $0.0228
        let expected = 0.20 * 1000.0 * (3600.0 / (365.0 * 86_400.0));
        assert!((p2.last_income_usd - expected).abs() < 1e-9);
    }

    #[test]
    fn rebalance_charges_cost_again() {
        let d1 = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let d2 = make_decision(Venue::Lighter, Venue::Hyperliquid, 0.15, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d1), 0.0);
        let p2 = t.accrue(1000, Some(&d2), 1.0);
        assert_eq!(p2.position_event, PositionEvent::Rebalanced);
        // Second open: cost_fraction × notional = 0.0015 × 1000 = $1.50
        assert!((p2.last_cost_usd - 1.5).abs() < 1e-12);
        // After two opens fees_paid = $3.00 cumulative
        assert!((p2.fees_paid_usd - 3.0).abs() < 1e-12);
    }

    #[test]
    fn long_hold_yields_positive_nav_at_realistic_spread() {
        // 18% pa spread, $100 notional, cost_fraction 0.0015.
        // Entry cost: 0.0015 × $100 = $0.15
        // Hourly income: 0.18 × 100 / 8760 ≈ $0.002055/h
        // Breakeven: 0.15 / 0.002055 ≈ 73 hours
        // At 200 hours we should be clearly net positive.
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.18, 100.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d), 0.0); // open: -$0.15
        for h in 1..=200 {
            t.accrue(h * 3_600_000, Some(&d), 3600.0);
        }
        assert!(
            t.nav_usd > 10_000.0,
            "200h hold should overtake entry cost, got {}",
            t.nav_usd
        );
        // Lower bound sanity: 200 × 0.002055 - 0.15 ≈ 0.261 so NAV ≈ 10_000.26
        assert!(
            t.nav_usd > 10_000.10,
            "200h hold should accrue > $0.10 net, got {}",
            t.nav_usd
        );
    }

    #[test]
    fn idle_tick_is_noop() {
        let mut t = NavTracker::new(10_000.0);
        let p = t.accrue(0, None, 10.0);
        assert_eq!(p.position_event, PositionEvent::Idle);
        assert!((t.nav_usd - 10_000.0).abs() < 1e-15);
    }

    #[test]
    fn held_through_gap_when_decision_missing() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d), 0.0);
        let p = t.accrue(1000, None, 1.0);
        assert_eq!(p.position_event, PositionEvent::HeldThroughGap);
    }

    #[test]
    fn nav_point_has_symbol_field() {
        let mut t = NavTracker::new_for_symbol("ETH", 5_000.0);
        let p = t.accrue(0, None, 0.0);
        assert_eq!(p.symbol, "ETH");
    }

    #[test]
    fn sub_cycle_accrual_scales_linearly_in_dt() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t1 = NavTracker::new(10_000.0);
        let mut t2 = NavTracker::new(10_000.0);
        t1.accrue(0, Some(&d), 0.0);
        t2.accrue(0, Some(&d), 0.0);
        // 3600s accrual in one big tick
        let p1 = t1.accrue(3_600_000, Some(&d), 3600.0);
        // Same interval in 60 small 60s ticks
        let mut ts = 60_000;
        let mut last_income = 0.0;
        for _ in 0..60 {
            let p = t2.accrue(ts, Some(&d), 60.0);
            last_income += p.last_income_usd;
            ts += 60_000;
        }
        // Sub-cycle decomposition must match the single-tick accrual within
        // floating-point tolerance (linearity of funding income in dt).
        assert!(
            (last_income - p1.last_income_usd).abs() < 1e-12,
            "60 × 60s accruals ({}) should equal 1 × 3600s accrual ({})",
            last_income,
            p1.last_income_usd
        );
    }

    #[test]
    fn fee_ledger_accumulates_across_rebalances() {
        let d1 = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let d2 = make_decision(Venue::Lighter, Venue::Hyperliquid, 0.15, 2000.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d1), 0.0); // fee 0.0015 × 1000 = $1.50
        let p = t.accrue(1000, Some(&d2), 0.0); // fee 0.0015 × 2000 = $3.00
                                                // Cumulative fees_paid = $4.50
        assert!((p.fees_paid_usd - 4.5).abs() < 1e-12);
    }

    #[test]
    fn fallback_cost_used_when_decision_cost_fraction_is_nan() {
        let mut d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        d.cost_fraction = f64::NAN;
        let mut t = NavTracker::new(10_000.0);
        let p = t.accrue(0, Some(&d), 0.0);
        // Fallback = 10 bps × $1000 = $1.00
        assert!((p.last_cost_usd - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mtm_with_fair_value_records_entry_baseline() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue_with_fair_value(0, Some(&d), 0.0, Some(100_000.0));
        // Next tick: fair value drifts +0.5% (price moved from 100k to 100.5k).
        // For a delta-neutral pair, MTM delta must remain ~0.
        let p = t.accrue_with_fair_value(3_600_000, Some(&d), 3600.0, Some(100_500.0));
        assert!(
            p.breakdown.mtm_delta_usd.abs() < 1e-9,
            "delta-neutral MTM must stay zero on price drift, got {}",
            p.breakdown.mtm_delta_usd
        );
    }

    #[test]
    fn nav_breakdown_sums_match_net_delta() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        let p = t.accrue_with_fair_value(0, Some(&d), 3600.0, Some(100_000.0));
        // breakdown.net_usd must equal last_accrual_usd exactly.
        assert!((p.breakdown.net_usd() - p.last_accrual_usd).abs() < 1e-12);
    }

    #[test]
    fn portfolio_nav_each_tracker_sees_full_nav() {
        //: portfolio starting NAV is $10k, each pair
        // notional is 1% of portfolio NAV ($100). The per-symbol tracker
        // is therefore initialized at the FULL portfolio NAV so that
        // `tracker.nav_usd` × 1% = $100 (not $10 with a sliced NAV).
        let symbols: Vec<String> = vec!["BTC".to_string(), "ETH".to_string()];
        let pf = PortfolioNav::new(10_000.0, &symbols);
        assert!((pf.trackers["BTC"].nav_usd - 10_000.0).abs() < 1e-12);
        assert!((pf.trackers["ETH"].nav_usd - 10_000.0).abs() < 1e-12);
        // Aggregate is NOT the sum of tracker NAVs (would double-count);
        // it's starting + Σ per-symbol cumulative accrual.
        assert!((pf.aggregate_nav_usd() - 10_000.0).abs() < 1e-12);
    }

    #[test]
    fn portfolio_nav_aggregate_point() {
        let symbols: Vec<String> = vec!["BTC".to_string(), "ETH".to_string()];
        let mut pf = PortfolioNav::new(10_000.0, &symbols);
        // Accrue on BTC: idle tick
        pf.accrue("BTC", 1000, None, 0.0);
        pf.accrue("ETH", 1000, None, 0.0);
        let agg = pf.snapshot_aggregate_point(1000);
        assert_eq!(agg.symbol, "AGGREGATE");
        assert_eq!(agg.event, "Tick");
        assert!((agg.nav_usd - 10_000.0).abs() < 1e-12);
    }
}
