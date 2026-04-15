use crate::venue::{FundingRate, OrderbookTop};

/// Push-based events from a venue adapter's WS/REST loop.
/// Consumed by bot-strategy or bot-cli's main loop.
#[derive(Debug, Clone)]
pub enum VenueEvent {
    FundingUpdate { venue: String, rate: FundingRate },
    OrderbookUpdate { venue: String, book: OrderbookTop },
    Connected { venue: String },
    Disconnected { venue: String, reason: String },
    Heartbeat { venue: String },
}
