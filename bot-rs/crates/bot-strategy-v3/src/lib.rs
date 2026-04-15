//! `bot-strategy-v3` — Rigorous statistical models + aurora-omega safety primitives.
//!
//! Phase 2a: self-contained stochastic functions ported from Python `strategy/stochastic.py`.
//! Week 1 addition: `funding_cycle_lock` — the iron law §1 gate (Wall 1).
//! No I/O, no async, no external runtime dependencies.

pub mod funding_cycle_lock;
pub mod stochastic;
