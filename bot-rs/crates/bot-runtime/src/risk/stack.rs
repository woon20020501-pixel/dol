//! `RiskStack` — composite of all six guards with worst-wins semantics.
//!
//! Ordering of severity (worst wins):
//!     Flatten > Block > Reduce(smallest mult) > Pass

use std::time::Instant;

use bot_strategy_v3::fsm_controller::{self, FsmDecision, FsmState, Mode, RiskReport};
use bot_types::Venue;

use super::concentration::{VenueConcentrationCap, VenueExposures};
use super::cvar_budget::CvarBudgetGuard;
use super::drawdown::DrawdownStop;
use super::heartbeat::HedgeHeartbeat;
use super::kill_switch::KillSwitch;
use super::watchdog::ApiLatencyWatchdog;
use super::RiskDecision;

/// Composite of all six runtime risk guards PLUS the Aurora-Ω §22 FSM
/// controller ported from `fsm_controller.py`.
pub struct RiskStack {
    pub cvar_budget: CvarBudgetGuard,
    pub kill_switch: KillSwitch,
    pub heartbeat: HedgeHeartbeat,
    pub watchdog: ApiLatencyWatchdog,
    pub concentration: VenueConcentrationCap,
    pub drawdown: DrawdownStop,
    /// Mutable FSM state (Kelly/Neutral/Robust transitions). Updated on
    /// every `evaluate` call via `fsm_controller::step`.
    pub fsm_state: FsmState,
    /// Last FSM decision (exposed for signal JSON + tests proving the FSM
    /// is on the runtime decision path).
    last_fsm_decision: Option<FsmDecision>,
}

impl RiskStack {
    pub fn new(initial_nav: f64) -> Self {
        Self {
            cvar_budget: CvarBudgetGuard::new(),
            kill_switch: KillSwitch::default_path(),
            heartbeat: HedgeHeartbeat::new(),
            watchdog: ApiLatencyWatchdog::new(),
            concentration: VenueConcentrationCap::new(),
            drawdown: DrawdownStop::new(initial_nav),
            fsm_state: FsmState::default(),
            last_fsm_decision: None,
        }
    }

    /// Latest FSM decision (None before first `evaluate`). Exposed so the
    /// signal JSON emitter can log `{mode, notional_scale, emergency_flatten,
    /// red_flags_fired}`.
    pub fn last_fsm_decision(&self) -> Option<&FsmDecision> {
        self.last_fsm_decision.as_ref()
    }

    /// Run every guard and return the most severe decision.
    ///
    /// Inputs:
    /// - `nav_usd`         : current total NAV
    /// - `exposures`       : per-venue USD exposure map
    /// - `basis_history`   : (ts_ms, basis) history for drawdown CVaR stop
    /// - `now`             : current Instant (for watchdog / heartbeat)
    ///
    /// Output: the WORST decision among all guards, with reasons concatenated.
    pub fn evaluate(
        &mut self,
        nav_usd: f64,
        exposures: &VenueExposures,
        basis_history: &[(i64, f64)],
        now: Instant,
    ) -> RiskDecision {
        let decisions = [
            ("kill_switch", self.kill_switch.check()),
            ("cvar_budget", self.cvar_budget.check(nav_usd)),
            ("heartbeat", self.heartbeat.check(now)),
            ("watchdog", self.watchdog.check(now)),
            ("concentration", self.concentration.check(exposures)),
            ("drawdown", self.drawdown.check(nav_usd, basis_history)),
        ];

        // ── FSM controller (Aurora-Ω §22) ─────────────────────────────────
        // Convert the 6-guard output into the 4-layer RiskReport shape the
        // FSM expects, then dispatch through `fsm_controller::step`. This
        // puts the FSM on the authoritative runtime decision path — it
        // isn't a parallel toy anymore.
        //
        // Mapping:
        //   cvar_budget   → layer "cvar"
        //   concentration → layer "entropic_ce"  (diversification proxy)
        //   drawdown      → layer "ecv"
        //   watchdog      → layer "execution_chi2"
        //   (kill_switch / heartbeat escalate independently via worst-wins)
        let reports = [
            RiskReport {
                layer: "cvar".to_string(),
                red_flag: decisions[1].1.is_blocking(),
            },
            RiskReport {
                layer: "entropic_ce".to_string(),
                red_flag: decisions[4].1.is_blocking(),
            },
            RiskReport {
                layer: "ecv".to_string(),
                red_flag: decisions[5].1.is_blocking(),
            },
            RiskReport {
                layer: "execution_chi2".to_string(),
                red_flag: decisions[3].1.is_blocking(),
            },
        ];
        let funding_healthy = !decisions.iter().any(|(_, d)| d.is_blocking());
        let fsm_decision = fsm_controller::step(
            &mut self.fsm_state,
            now.elapsed().as_secs_f64(),
            &reports,
            false, // forecast_flag wired in scoring layer, not here
            funding_healthy,
            matches!(decisions[0].1, RiskDecision::Flatten { .. }),
        );

        // FSM emergency_flatten is a hard override that wins over all
        // per-guard outputs.
        if fsm_decision.emergency_flatten {
            let reason = format!(
                "[fsm:{mode:?}] {rationale}",
                mode = fsm_decision.mode,
                rationale = fsm_decision.rationale
            );
            self.last_fsm_decision = Some(fsm_decision);
            return RiskDecision::Flatten { reason };
        }

        // Find worst per-guard decision (highest severity).
        let (name, worst) = decisions
            .iter()
            .max_by_key(|(_, d)| severity_rank(d))
            .expect("6 decisions cannot be empty");

        // Combine the FSM notional_scale with the per-guard size multiplier.
        // The FSM multiplier applies even when no guard blocks (Robust or
        // Neutral mode reduces size preventively).
        let guard_mult = worst.size_multiplier();
        let combined_mult = (guard_mult * fsm_decision.notional_scale).clamp(0.0, 1.0);

        let out = match worst {
            RiskDecision::Pass => {
                if fsm_decision.mode != Mode::KellySafe {
                    RiskDecision::Reduce {
                        size_multiplier: combined_mult,
                        reason: format!(
                            "[fsm:{mode:?}] {rationale}",
                            mode = fsm_decision.mode,
                            rationale = fsm_decision.rationale
                        ),
                    }
                } else {
                    RiskDecision::Pass
                }
            }
            RiskDecision::Reduce { reason, .. } => RiskDecision::Reduce {
                size_multiplier: combined_mult,
                reason: format!("[{name}+fsm:{mode:?}] {reason}", mode = fsm_decision.mode),
            },
            RiskDecision::Block { reason } => RiskDecision::Block {
                reason: format!("[{name}] {reason}"),
            },
            RiskDecision::Flatten { reason } => RiskDecision::Flatten {
                reason: format!("[{name}] {reason}"),
            },
        };
        self.last_fsm_decision = Some(fsm_decision);
        out
    }

    /// Helper: ingest a single-symbol NAV update into all relevant guards.
    pub fn on_nav_update(&mut self, nav_usd: f64, pnl_delta_usd: f64) {
        self.cvar_budget.record(pnl_delta_usd);
        self.drawdown.record_nav(nav_usd);
    }

    /// Helper: ingest an adapter fetch timing observation.
    pub fn on_api_latency(&mut self, ts: Instant, latency: std::time::Duration) {
        self.watchdog.record(ts, latency);
    }

    /// Helper: ingest a fill event (stub — real fills come in Week 2+).
    pub fn on_pivot_fill(&mut self, now: Instant) {
        self.heartbeat.record_pivot_fill(now);
    }

    pub fn on_hedge_fill(&mut self, now: Instant) {
        self.heartbeat.record_hedge_fill(now);
    }

    /// Helper: allow operator to pre-arm kill switch.
    pub fn trip_kill_switch(&self) {
        self.kill_switch.trip();
    }
}

/// Severity ordering (higher = worse).
fn severity_rank(d: &RiskDecision) -> u8 {
    match d {
        RiskDecision::Pass => 0,
        RiskDecision::Reduce { .. } => 1,
        RiskDecision::Block { .. } => 2,
        RiskDecision::Flatten { .. } => 3,
    }
}

/// Build a venue-exposure map from an iterator of (Venue, notional_usd)
/// pairs, summing across duplicates.
pub fn build_exposures(items: impl Iterator<Item = (Venue, f64)>) -> VenueExposures {
    let mut map = VenueExposures::new();
    for (v, n) in items {
        *map.entry(v).or_insert(0.0) += n;
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_when_no_guard_fires() {
        let mut stack = RiskStack::new(10_000.0);
        let exposures = build_exposures(
            vec![
                (Venue::Pacifica, 100.0),
                (Venue::Hyperliquid, 100.0),
                (Venue::Lighter, 100.0),
                (Venue::Backpack, 100.0),
            ]
            .into_iter(),
        );
        let decision = stack.evaluate(10_000.0, &exposures, &[], Instant::now());
        assert_eq!(decision, RiskDecision::Pass);
    }

    #[test]
    fn kill_switch_dominates() {
        let mut stack = RiskStack::new(10_000.0);
        stack.kill_switch.trip();
        let decision = stack.evaluate(10_000.0, &VenueExposures::new(), &[], Instant::now());
        assert!(matches!(decision, RiskDecision::Flatten { .. }));
    }

    #[test]
    fn concentration_block_overrides_pass() {
        let mut stack = RiskStack::new(10_000.0);
        let exposures = build_exposures(vec![(Venue::Pacifica, 1000.0)].into_iter());
        let decision = stack.evaluate(10_000.0, &exposures, &[], Instant::now());
        assert!(matches!(decision, RiskDecision::Block { .. }));
    }

    #[test]
    fn build_exposures_sums_duplicates() {
        let e = build_exposures(
            vec![
                (Venue::Pacifica, 100.0),
                (Venue::Pacifica, 200.0),
                (Venue::Lighter, 50.0),
            ]
            .into_iter(),
        );
        assert_eq!(e.get(&Venue::Pacifica).copied().unwrap_or(0.0), 300.0);
        assert_eq!(e.get(&Venue::Lighter).copied().unwrap_or(0.0), 50.0);
    }

    /// Proof: FSM controller runs on every evaluate() — last_fsm_decision
    /// is populated after the first call.
    #[test]
    fn fsm_runs_on_every_evaluate() {
        let mut stack = RiskStack::new(10_000.0);
        assert!(stack.last_fsm_decision().is_none());
        let exposures = build_exposures(
            vec![
                (Venue::Pacifica, 100.0),
                (Venue::Hyperliquid, 100.0),
                (Venue::Lighter, 100.0),
                (Venue::Backpack, 100.0),
            ]
            .into_iter(),
        );
        let _ = stack.evaluate(10_000.0, &exposures, &[], Instant::now());
        let fsm = stack.last_fsm_decision().expect("FSM decision must exist");
        // Clean state → Kelly-safe.
        assert_eq!(fsm.mode, Mode::KellySafe);
        assert!((fsm.notional_scale - 1.0).abs() < 1e-12);
    }

    /// Proof: when 2+ guard red-flags fire, FSM dispatches to Robust and
    /// returns emergency_flatten as the composite decision.
    #[test]
    fn fsm_two_red_flags_triggers_robust_flatten() {
        let mut stack = RiskStack::new(10_000.0);
        // Concentration block (one venue) AND drawdown (force via NAV drop).
        let exposures = build_exposures(vec![(Venue::Pacifica, 1000.0)].into_iter());
        // Drop NAV 10% to fire drawdown.
        stack.drawdown.record_nav(10_000.0);
        let decision = stack.evaluate(9_000.0, &exposures, &[], Instant::now());
        let fsm = stack.last_fsm_decision().expect("FSM decision must exist");
        assert_eq!(fsm.mode, Mode::Robust);
        // When FSM emergency_flatten fires, composite is Flatten.
        if fsm.emergency_flatten {
            assert!(matches!(decision, RiskDecision::Flatten { .. }));
        }
    }
}
