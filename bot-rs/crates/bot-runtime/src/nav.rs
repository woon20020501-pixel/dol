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

/// Round-trip cost charged on every position open or rebalance.
///
/// 10 bps ≈ taker fee on both legs (≈ 3-5 bps each on DEXes) + bid-ask spread
///     + slippage estimate. This is a conservative default — live calibration
///     via the `slippage_calibration` framework module is a v1+ task.
pub const ROUND_TRIP_COST_BPS: f64 = 10.0;

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

/// Identity of an open position, used to detect rebalances.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenPosition {
    long_venue: Venue,
    short_venue: Venue,
    symbol: String,
}

impl OpenPosition {
    fn matches(&self, d: &PairDecision) -> bool {
        self.long_venue == d.long_venue
            && self.short_venue == d.short_venue
            && self.symbol == d.symbol
    }

    fn from_decision(d: &PairDecision) -> Self {
        Self {
            long_venue: d.long_venue,
            short_venue: d.short_venue,
            symbol: d.symbol.clone(),
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
            current: None,
        }
    }

    /// Apply a decision (or `None`) to the NAV.
    ///
    /// # Accrual model
    ///
    /// Open / rebalance: pay `ROUND_TRIP_COST_BPS` of notional, once.
    /// Hold: accrue `spread_annual × notional × dt / (365 × 86400)`.
    /// Idle / gap: no change.
    pub fn accrue(
        &mut self,
        ts_ms: i64,
        decision: Option<&PairDecision>,
        dt_seconds: f64,
    ) -> NavPoint {
        let (last_income_usd, last_cost_usd, event) = match (decision, &self.current) {
            (Some(d), None) => {
                // First-time open: pay round-trip cost + accrue first-tick funding.
                let cost = ROUND_TRIP_COST_BPS * 1e-4 * d.notional_usd;
                let income = funding_income(d, dt_seconds);
                self.current = Some(OpenPosition::from_decision(d));
                (income, cost, PositionEvent::Opened)
            }
            (Some(d), Some(pos)) if pos.matches(d) => {
                // Hold: funding only, no cost.
                let income = funding_income(d, dt_seconds);
                (income, 0.0, PositionEvent::Held)
            }
            (Some(d), Some(_)) => {
                // Rebalance to a new pair: pay cost again + accrue first-tick funding.
                let cost = ROUND_TRIP_COST_BPS * 1e-4 * d.notional_usd;
                let income = funding_income(d, dt_seconds);
                self.current = Some(OpenPosition::from_decision(d));
                (income, cost, PositionEvent::Rebalanced)
            }
            (None, None) => (0.0, 0.0, PositionEvent::Idle),
            (None, Some(_)) => (0.0, 0.0, PositionEvent::HeldThroughGap),
        };

        let last_accrual_usd = last_income_usd - last_cost_usd;
        self.nav_usd += last_accrual_usd;
        self.cumulative_accrual_usd += last_accrual_usd;

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
            last_income_usd,
            last_cost_usd,
            cumulative_accrual_usd: self.cumulative_accrual_usd,
            position_event: event,
            event: event_label,
            income_usd: last_income_usd,
            cost_usd: last_cost_usd,
        };
        self.history.push(point.clone());
        point
    }
}

/// Portfolio-level NAV tracker: one `NavTracker` per symbol.
///
/// **Accounting model:** each per-symbol `NavTracker`
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
    fn open_charges_roundtrip_cost_once() {
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.20, 1000.0);
        let mut t = NavTracker::new(10_000.0);
        let p1 = t.accrue(0, Some(&d), 0.0); // open with dt=0 → no income yet
        assert_eq!(p1.position_event, PositionEvent::Opened);
        // Cost = 10 bps × $1000 = $1.00
        assert!((p1.last_cost_usd - 1.0).abs() < 1e-12);
        assert!((p1.last_income_usd - 0.0).abs() < 1e-12);
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
        assert!((p2.last_cost_usd - 1.0).abs() < 1e-12);
    }

    #[test]
    fn long_hold_yields_positive_nav_at_realistic_spread() {
        // 18% pa spread, $100 notional.
        // Entry cost: 10 bps × $100 = $0.10
        // Hourly income: 0.18 × 100 / 8760 ≈ $0.002055/h
        // Breakeven: 0.10 / 0.002055 ≈ 48.66 hours
        // So at 100 hours we should be clearly net positive.
        let d = make_decision(Venue::Pacifica, Venue::Backpack, 0.18, 100.0);
        let mut t = NavTracker::new(10_000.0);
        t.accrue(0, Some(&d), 0.0); // open: -$0.10
                                    // Hold for 100 hours of 1-hour ticks.
        for h in 1..=100 {
            t.accrue(h * 3_600_000, Some(&d), 3600.0);
        }
        assert!(
            t.nav_usd > 10_000.0,
            "100h hold should overtake entry cost, got {}",
            t.nav_usd
        );
        // Lower bound sanity: cumulative income after 100h minus cost
        // = 100 × 0.002055 - 0.10 ≈ 0.1055, so NAV ≈ 10_000.10
        assert!(
            t.nav_usd > 10_000.05,
            "100h hold should accrue > $0.05 net, got {}",
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
    fn portfolio_nav_each_tracker_sees_full_nav() {
        // Per the accounting model: portfolio starting NAV is $10k, each pair
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
