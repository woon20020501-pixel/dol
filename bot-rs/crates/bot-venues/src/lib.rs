//! bot-venues — Pacifica + Lighter venue adapters.
//!
//! M2 scope: WS + REST adapters with reconnect, circuit breaker,
//! and unified Venue trait.

pub mod config;
pub mod error;
pub mod event;
pub mod lighter;
pub mod net;
pub mod pacifica;
pub mod venue;

pub use config::{LighterConfig, PacificaConfig};
pub use error::VenueError;
pub use event::VenueEvent;
pub use venue::{
    Balance, FundingRate, OrderbookLevel, OrderbookTop, Position, PositionSide, Venue,
};
