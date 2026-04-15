//! `bot-adapters` — venue adapter abstraction + real and dryrun adapters.
//!
//! # Crate layout
//!
//! - `venue`    — `VenueAdapter` trait, `VenueSnapshot`, `PositionView`,
//!   `OrderIntent`, `FillReport`, `AdapterError`
//! - `pacifica` — `PacificaReadOnlyAdapter` wrapping `bot-venues::pacifica::rest`
//! - `dryrun`   — `DryRunVenueAdapter` loading fixture JSON snapshots
//!
//! # Step A scope
//!
//! Only read-only data fetching and dry-run order simulation are implemented.
//! Tick loop, signal JSON emission, and CLI wiring are deferred to Step B.
//! Live order signing / submission does NOT exist in this crate.

pub mod dryrun;
pub mod pacifica;
pub mod pacifica_auth;
pub mod venue;

pub use venue::{
    AdapterError, FillReport, OrderIntent, OrderKind, PositionView, VenueAdapter, VenueSnapshot,
};
