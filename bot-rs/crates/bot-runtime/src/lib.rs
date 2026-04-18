//! `bot-runtime` — Week 1 Step B hackathon demo runtime.
//!
//! Wires a minimal tick loop, decision logger, simulated NAV tracker, and
//! signal JSON emitter against the `bot-adapters` `VenueAdapter` trait.
//!
//! **This crate is a demo scaffold.** Most framework modules
//! (`fair_value_oracle`, `fsm_controller`, `forecast_scoring`, `risk_stack`,
//! …) are stubbed in the signal JSON output and will be replaced in
//! Week 2+ with real Rust ports. **Exception:** `funding_cycle_lock` is
//! already ported and lives in `bot-strategy-v3`; every decision path on
//! this runtime goes through `cycle_lock::CycleLockRegistry::enforce_decision`
//! so that I-LOCK is honored from day one (no escape hatch for the
//! live-submission path to plug later).

pub mod adapter_health;
pub mod backpressure;
pub mod clock;
pub mod cycle_lock;
pub mod decision;
pub mod fair_value;
pub mod history;
pub mod live_gate;
pub mod metrics;
pub mod nav;
pub mod risk;
pub mod scoring;
pub mod signal;
pub mod tick;
pub mod tracing_redact;
