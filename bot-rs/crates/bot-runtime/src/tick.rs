//! `TickEngine` — minimal tick loop for the hackathon demo.
//!
//! Each call to `run_one_tick` for a symbol:
//! 1. Fetches snapshots in parallel from every registered adapter.
//! 2. Drops adapters that errored (WARN log, non-fatal).
//! 3. Computes weighted fair value via `fair_value::compute_weighted_fair_value`.
//! 4. Calls `decision::decide(snapshots, nav)` → `Option<PairDecision>`.
//! 5. Accrues NAV via the caller-provided `NavTracker`.
//! 6. Returns a `TickOutput` with all the above.
//!
//! Signal JSON emission and NAV logging are left to the caller so that
//! `run_one_tick` is independently testable without filesystem I/O.

use std::collections::BTreeMap;
use std::sync::Arc;

use tracing::{info, warn};

use bot_adapters::venue::{VenueAdapter, VenueSnapshot};
use bot_types::Venue;

use crate::adapter_health::{AdapterHealthRegistry, SymbolHealth};
use crate::cycle_lock::{CycleLockRegistry, EnforceOutcome};
use crate::decision::{self, PairDecision};
use crate::fair_value::{self, FairValue};
use crate::history::FundingHistoryRegistry;
use crate::nav::NavTracker;
use crate::risk::{self, RiskDecision, RiskStack};
use crate::scoring::{self, ForecastScore, ForecastVerdict, ScoringInputs};

// ─────────────────────────────────────────────────────────────────────────────
// Core output types
// ─────────────────────────────────────────────────────────────────────────────

/// One tick's output. Written to the signal JSON directory by the caller.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TickOutput {
    /// Tick timestamp in Unix milliseconds.
    pub ts_ms: i64,
    /// Symbol that was evaluated.
    pub symbol: String,
    /// Snapshots from venues that returned data (erroring venues excluded).
    pub snapshots: Vec<VenueSnapshot>,
    /// Computed fair value across contributing venues.
    pub fair_value: FairValue,
    /// The raw proposed decision from `decision::decide` — BEFORE lock
    /// enforcement. Kept for telemetry / "would have wanted" visibility.
    pub proposed_decision: Option<PairDecision>,
    /// Effective decision — what the bot ACTS on. Equals `proposed_decision`
    /// only when the lock opened a new cycle; otherwise it's the locked
    /// decision that was held.
    pub decision: Option<PairDecision>,
    /// Raw enforce result from `funding_cycle_lock::enforce` (Python-parity
    /// shape). Populated on every tick.
    pub cycle_lock: CycleLockInfo,
    /// Adapter health snapshot for this symbol AFTER this tick's fetch
    /// attempts were recorded. Rolled up into signal JSON diagnostics so
    /// the dashboard can flag flaky symbols.
    pub adapter_health: SymbolHealth,
    /// NAV after applying the decision accrual.
    pub nav_after: f64,
    /// Composite decision from the 6-guard runtime risk stack. Determines
    /// whether the effective decision was passed, reduced, blocked, or
    /// triggered a flatten. Fed into signal JSON `risk_stack` payload.
    pub risk_decision: RiskDecision,
    /// Size multiplier applied to the effective decision's notional
    /// (1.0 = pass, 0.0 = block/flatten, ∈ (0, 1) = proportional reduce).
    pub risk_size_multiplier: f64,
    /// Forecast score — regime classification + OU fit + break-even hold +
    /// Bernstein leverage bound + expected residual income. Feeds signal JSON
    /// `forecast_scoring` section (no longer stubbed once this is populated).
    pub forecast: ForecastScore,
}

/// Compact cycle-lock telemetry for the tick output and signal JSON.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CycleLockInfo {
    /// True if a cycle is currently locked for this symbol.
    pub locked: bool,
    /// Cycle index at the time of this tick.
    pub cycle_index: i64,
    /// Locked funding-leg direction (`+1`/`-1`/`0`).
    pub h_c: i8,
    /// Locked notional in USD.
    pub n_c: f64,
    /// Seconds remaining until the current cycle boundary.
    pub seconds_to_cycle_end: f64,
    /// True if `emergency_override=true` was used this tick.
    pub emergency_override: bool,
    /// True if this tick opened a fresh cycle.
    pub opened_new_cycle: bool,
    /// True if a proposed pair flip or rebalance was blocked.
    pub proposed_was_blocked: bool,
}

impl CycleLockInfo {
    /// Convert an `EnforceOutcome` into the compact telemetry shape.
    pub fn from_outcome(
        outcome: &EnforceOutcome,
        state: Option<bot_strategy_v3::funding_cycle_lock::CycleState>,
        now_s: f64,
    ) -> Self {
        let (locked, cycle_index, h_c, n_c) = match state {
            Some(s) => (true, s.cycle_index, s.h_c, s.n_c),
            None => (false, 0, 0, 0.0),
        };
        let cycle_seconds = state
            .map(|s| s.cycle_seconds)
            .unwrap_or(bot_strategy_v3::funding_cycle_lock::DEFAULT_CYCLE_SECONDS);
        let seconds_to_end =
            bot_strategy_v3::funding_cycle_lock::seconds_to_cycle_end(now_s, cycle_seconds);
        Self {
            locked,
            cycle_index,
            h_c,
            n_c,
            seconds_to_cycle_end: seconds_to_end,
            emergency_override: outcome.raw.emergency_override_used,
            opened_new_cycle: outcome.opened_new_cycle,
            proposed_was_blocked: outcome.pair_flip_blocked || outcome.raw.proposed_was_blocked,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TickEngine
// ─────────────────────────────────────────────────────────────────────────────

/// Tick engine: one instance per demo run.
///
/// Holds a map of venue → adapter and the list of symbols to evaluate.
/// `run_one_tick` is safe to call concurrently for different symbols.
pub struct TickEngine {
    /// Adapters keyed by venue. At least one must be provided.
    pub adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>>,
    /// Symbols to evaluate each tick. For the demo, operator-specified.
    pub symbols: Vec<String>,
    /// Minimum annualized spread to emit a decision (default 2 bps = 0.0002).
    pub min_spread_threshold: f64,
}

impl TickEngine {
    /// Construct a new engine with the given adapters and symbols.
    ///
    /// `min_spread_threshold` defaults to **2% annualized (200 bps)** — any
    /// candidate pair whose spread doesn't clear that gate is ignored. Rationale:
    /// at 10 bps entry cost per pair, the per-tick breakeven window for a 1%
    /// spread pair is on the order of 800 hours — too long to be worth the
    /// capital slot in a 46-pair universe. The 2% floor mirrors the bot's
    /// production-like behavior of skipping thin spreads rather than trading
    /// them. Combined with `decision::REBALANCE_HYSTERESIS_FACTOR` (1.5×) and
    /// the absolute floor (100 bps), this eliminates demo churn from
    /// live-funding-rate noise on low-spread pairs.
    pub fn new(adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>>, symbols: Vec<String>) -> Self {
        Self {
            adapters,
            symbols,
            min_spread_threshold: 0.02,
        }
    }

    /// Run a single tick for `symbol`.
    ///
    /// Steps:
    /// 1. **Parallel fetch** — all adapters queried concurrently. Per-adapter
    ///    errors are non-fatal (WARN + skip).
    /// 2. **Fair value** — `fair_value::compute_weighted_fair_value`.
    /// 3. **Proposed decision** — `decision::decide` produces the raw
    ///    cross-venue proposal.
    /// 4. **Iron law enforcement (I-LOCK)** — the proposal is threaded
    ///    through `cycle_lock_registry.enforce_decision`. If a cycle is
    ///    active, the proposal may be held at the locked values. Pair
    ///    flips / rebalances are blocked and logged.
    /// 5. **NAV accrual** — uses the **effective** decision returned by
    ///    the lock registry, not the proposal.
    /// 6. **Return** — `TickOutput` with proposed, effective, and lock info.
    ///
    /// Signal JSON is NOT written here — the caller writes it BEFORE any
    /// adapter submission (integration-spec §5.3 ordering rule).
    ///
    /// The 9 arguments are all independent sub-registries that must be
    /// passed in by the caller (each corresponds to a distinct Priority
    /// deliverable: NAV, cycle-lock, adapter health, risk stack, funding
    /// history, plus the scalar time inputs). Bundling into a struct would
    /// just rename the 9 fields and not reduce the actual dependency count.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(
        name = "run_one_tick",
        skip_all,
        fields(symbol = %symbol, now_ms, dt_s = dt_seconds)
    )]
    pub async fn run_one_tick(
        &self,
        symbol: &str,
        nav_tracker: &mut NavTracker,
        cycle_lock_registry: &mut CycleLockRegistry,
        adapter_health: &mut AdapterHealthRegistry,
        risk_stack: &mut RiskStack,
        history: &mut FundingHistoryRegistry,
        now_ms: i64,     // simulated Unix milliseconds (from SimulatedClock::now_ms())
        dt_seconds: f64, // simulated elapsed seconds since last tick
    ) -> anyhow::Result<TickOutput> {
        let ts_ms = now_ms;
        let now_s = now_ms as f64 / 1000.0;
        let now_instant = std::time::Instant::now();

        // ── Step 1: Parallel fetch with per-adapter latency timing ────────
        let mut fetch_futures = Vec::with_capacity(self.adapters.len());
        for (venue, adapter) in &self.adapters {
            let symbol_owned = symbol.to_string();
            let venue = *venue;
            let adapter = Arc::clone(adapter);
            fetch_futures.push(async move {
                let t_start = std::time::Instant::now();
                let result = adapter.fetch_snapshot(&symbol_owned).await;
                let latency = t_start.elapsed();
                (venue, result, latency)
            });
        }

        let results = futures_util::future::join_all(fetch_futures).await;

        // ── Step 2: Drop errors, collect snapshots, record health + latency ─
        let mut snapshots: Vec<VenueSnapshot> = Vec::new();
        let mut failure_reasons: Vec<(Venue, String)> = Vec::new();
        for (venue, result, latency) in results {
            // Feed latency to the Pacifica watchdog (per I-PAC-WATCH spec —
            // the guard only monitors Pacifica; other venues' latency is
            // recorded but not gated on).
            if venue == Venue::Pacifica {
                risk_stack.on_api_latency(now_instant, latency);
            }
            match result {
                Ok(snap) => {
                    info!(
                        venue = ?venue,
                        symbol = %symbol,
                        mid_price = snap.mid_price,
                        funding_annual = snap.funding_rate_annual.0,
                        latency_ms = latency.as_millis(),
                        "snapshot fetched"
                    );
                    snapshots.push(snap);
                }
                Err(e) => {
                    let reason = e.to_string();
                    warn!(
                        venue = ?venue,
                        symbol = %symbol,
                        error = %reason,
                        latency_ms = latency.as_millis(),
                        "adapter fetch failed — skipping venue this tick"
                    );
                    failure_reasons.push((venue, reason));
                }
            }
        }

        // Record per-symbol adapter health telemetry.
        let failures_ref: Vec<(Venue, &str)> = failure_reasons
            .iter()
            .map(|(v, r)| (*v, r.as_str()))
            .collect();
        adapter_health.record_tick(symbol, &failures_ref);
        let health_snapshot = adapter_health.health_for(symbol);
        if health_snapshot.is_degraded {
            warn!(
                symbol = %symbol,
                consecutive_failures = health_snapshot.consecutive_failures,
                total_failures = health_snapshot.total_failures,
                "adapter_health: symbol flagged DEGRADED (≥3 consecutive fetch failures)"
            );
        }

        // Record this tick's venue observations into history (for OU/ADF fits).
        let history_obs: Vec<(Venue, f64)> = snapshots
            .iter()
            .map(|s| (s.venue, s.funding_rate_annual.0))
            .collect();
        history.record_tick(symbol, ts_ms, &history_obs);

        // ── Step 3: Fair value ────────────────────────────────────────────
        let fair_value = fair_value::compute_weighted_fair_value(&snapshots);
        info!(
            symbol = %symbol,
            p_star = fair_value.p_star,
            healthy = fair_value.healthy,
            num_venues = fair_value.contributing_venues.len(),
            "fair value computed"
        );

        // ── Step 4: Proposed decision ─────────────────────────────────────
        // Pull the currently-held pair (if any) from the cycle-lock registry
        // so `decide()` can apply rebalance hysteresis.
        let held_for_hysteresis = cycle_lock_registry.locked_decision_for(symbol).cloned();
        let proposed = decision::decide(
            &snapshots,
            nav_tracker.nav_usd,
            self.min_spread_threshold,
            held_for_hysteresis.as_ref(),
        );

        // ── Step 4b: Forecast scoring (regime, OU, breakeven, Bernstein) ──
        // Uses the accumulated history; returns Insufficient on <50 samples.
        let spread_series = history.spread_series(symbol);
        let spread_values = history.spread_values(symbol);
        let current_spread = proposed.as_ref().map(|d| d.spread_annual).unwrap_or(0.0);
        let cost_fraction = proposed.as_ref().map(|d| d.cost_fraction).unwrap_or(0.0);
        let (delta_bound, sigma_per_h) =
            scoring::infer_spread_dynamics(&spread_series, dt_seconds / 3600.0);
        let forecast = scoring::score(ScoringInputs {
            spread_series: &spread_series,
            spread_values: &spread_values,
            current_spread_annual: current_spread,
            cost_fraction,
            dt_hours: (dt_seconds / 3600.0).max(1e-6),
            mmr: 0.03, // common DEX MMR floor; will be per-venue once exchange metadata is live
            delta_bound_per_h: delta_bound,
            sigma_per_h,
        });

        // Apply forecast verdict to the proposed notional. Admit=1.0,
        // Reduce=0.5, Reject=0.0. When history is Insufficient the verdict
        // is Admit (bootstrap). The CYCLE_LOCK still sees the *scaled*
        // notional so the lock won't hold an oversized stale position.
        let forecast_scale = scoring::verdict_size_scale(forecast.verdict);
        let proposed = proposed.map(|mut d| {
            d.notional_usd *= forecast_scale;
            d
        });
        if forecast.verdict != ForecastVerdict::Admit {
            info!(
                symbol = %symbol,
                regime = ?forecast.regime,
                verdict = ?forecast.verdict,
                tau_be_h = ?forecast.tau_be_hours,
                leverage = ?forecast.leverage_bound,
                "forecast: reduced/rejected by admission gate"
            );
        }

        // ── Step 5: Iron law enforcement (I-LOCK) ─────────────────────────
        // Prefer the typestate-safe path: project the runtime symbol
        // string into a compile-time marker via `with_symbol_marker!`.
        // Inside that scope, `PairDecision::try_typed::<M>()` attaches
        // the marker and `enforce_decision_typed::<M>()` uses `M::NAME`
        // as the registry key — so `(symbol, decision)` can no longer
        // desynchronise.
        //
        // If the runtime symbol isn't in the marker whitelist (e.g. a
        // newly-listed coin that hasn't been added to `declare_markers!`
        // yet) we fall back to the untyped path with a warning so the
        // tick loop doesn't block.
        let outcome: EnforceOutcome = bot_types::with_symbol_marker!(symbol, |M| {
            let typed = proposed.as_ref().and_then(|d| d.try_typed::<M>());
            cycle_lock_registry.enforce_decision_typed::<M>(now_s, typed, false)
        })
        .unwrap_or_else(|| {
            tracing::warn!(
                symbol = %symbol,
                "cycle_lock: no compile-time marker for symbol — using untyped enforce"
            );
            cycle_lock_registry.enforce_decision(symbol, now_s, proposed.as_ref(), false)
        });
        let effective = outcome.effective.clone();

        // Log enforcement visibility.
        if outcome.opened_new_cycle {
            if let Some(d) = &effective {
                info!(
                    symbol = %symbol,
                    h_eff = outcome.raw.h_eff,
                    n_eff = outcome.raw.n_eff,
                    long = ?d.long_venue,
                    short = ?d.short_venue,
                    "cycle_lock: opened new cycle"
                );
            }
        } else if outcome.pair_flip_blocked {
            warn!(
                symbol = %symbol,
                h_eff = outcome.raw.h_eff,
                "I-LOCK: proposed pair would flip mid-cycle — holding locked pair"
            );
        } else if effective.is_some() && proposed.is_some() {
            info!(
                symbol = %symbol,
                h_eff = outcome.raw.h_eff,
                n_eff = outcome.raw.n_eff,
                "cycle_lock: holding locked direction"
            );
        }

        // ── Step 6: NAV accrual (uses effective, not proposed) ────────────
        // Pass the live fair_value so the tracker updates its MTM anchor
        // and the NavBreakdown includes basis-P&L telemetry.
        let fv_for_mtm = if fair_value.healthy {
            Some(fair_value.p_star)
        } else {
            None
        };
        let nav_point =
            nav_tracker.accrue_with_fair_value(ts_ms, effective.as_ref(), dt_seconds, fv_for_mtm);

        // Record NAV delta into CVaR + drawdown guards (Rockafellar-Uryasev
        // rolling window for CVaR_99 vs budget99 frac).
        risk_stack.on_nav_update(nav_point.nav_usd, nav_point.delta_usd);

        // ── Step 7: Risk stack evaluation (6-guard composite) ────────────
        let exposures = risk::build_exposures(effective.as_ref().into_iter().flat_map(|d| {
            std::iter::once((d.long_venue, d.notional_usd))
                .chain(std::iter::once((d.short_venue, d.notional_usd)))
        }));
        let risk_decision = risk_stack.evaluate(
            nav_point.nav_usd,
            &exposures,
            &[], // basis_history wired in P7 step
            now_instant,
        );
        let risk_size_multiplier = risk_decision.size_multiplier();

        // Apply the worst-decision size multiplier to the effective decision.
        // Flatten/Block → notional forced to 0; Reduce → proportional.
        let effective = effective.map(|mut d| {
            d.notional_usd *= risk_size_multiplier;
            d
        });
        if !matches!(risk_decision, RiskDecision::Pass) {
            warn!(
                symbol = %symbol,
                ?risk_decision,
                size_multiplier = risk_size_multiplier,
                "risk_stack: non-pass decision applied"
            );
        }

        let cycle_state = cycle_lock_registry.state_for(symbol);
        let lock_info = CycleLockInfo::from_outcome(&outcome, cycle_state, now_s);

        Ok(TickOutput {
            ts_ms,
            symbol: symbol.to_string(),
            snapshots,
            fair_value,
            proposed_decision: proposed,
            decision: effective,
            cycle_lock: lock_info,
            adapter_health: health_snapshot,
            nav_after: nav_point.nav_usd,
            risk_decision,
            risk_size_multiplier,
            forecast,
        })
    }
}
