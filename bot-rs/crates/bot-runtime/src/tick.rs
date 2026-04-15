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
use crate::nav::NavTracker;

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
    pub async fn run_one_tick(
        &self,
        symbol: &str,
        nav_tracker: &mut NavTracker,
        cycle_lock_registry: &mut CycleLockRegistry,
        adapter_health: &mut AdapterHealthRegistry,
        now_ms: i64,     // simulated Unix milliseconds (from SimulatedClock::now_ms())
        dt_seconds: f64, // simulated elapsed seconds since last tick
    ) -> anyhow::Result<TickOutput> {
        let ts_ms = now_ms;
        let now_s = now_ms as f64 / 1000.0;

        // ── Step 1: Parallel fetch ────────────────────────────────────────
        let mut fetch_futures = Vec::with_capacity(self.adapters.len());
        for (venue, adapter) in &self.adapters {
            let symbol_owned = symbol.to_string();
            let venue = *venue;
            let adapter = Arc::clone(adapter);
            fetch_futures.push(async move {
                let result = adapter.fetch_snapshot(&symbol_owned).await;
                (venue, result)
            });
        }

        let results = futures_util::future::join_all(fetch_futures).await;

        // ── Step 2: Drop errors, collect snapshots, record health ────────
        let mut snapshots: Vec<VenueSnapshot> = Vec::new();
        let mut failure_reasons: Vec<(Venue, String)> = Vec::new();
        for (venue, result) in results {
            match result {
                Ok(snap) => {
                    info!(
                        venue = ?venue,
                        symbol = %symbol,
                        mid_price = snap.mid_price,
                        funding_annual = snap.funding_rate_annual.0,
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

        // ── Step 5: Iron law enforcement (I-LOCK) ─────────────────────────
        let outcome: EnforceOutcome = cycle_lock_registry.enforce_decision(
            symbol,
            now_s,
            proposed.as_ref(),
            false, // emergency_override
        );
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
        let nav_point = nav_tracker.accrue(ts_ms, effective.as_ref(), dt_seconds);

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
        })
    }
}
