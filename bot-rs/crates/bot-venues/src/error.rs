use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VenueError {
    // ── Transient (trigger reconnect / retry) ──────────────────────
    #[error("ws connection failed: {0}")]
    WsConnect(String),

    #[error("ws read timeout after {0:?}")]
    WsTimeout(Duration),

    #[error("ws send failed: {0}")]
    WsSend(String),

    #[error("rest request failed: {0}")]
    RestRequest(String),

    // ── Parse (log + skip, don't reconnect) ────────────────────────
    #[error("message parse failed: {0}")]
    ParseError(String),

    // ── Fatal (stop adapter, surface to operator) ──────────────────
    #[error("authentication failed: {0}")]
    AuthError(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("venue returned error: code={code} {message}")]
    ApiError { code: String, message: String },

    #[error("config missing: {0}")]
    ConfigMissing(String),

    #[error("adapter shutdown")]
    Shutdown,
}

impl VenueError {
    /// Whether this error is transient and should trigger a retry.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::WsConnect(_) | Self::WsTimeout(_) | Self::WsSend(_) | Self::RestRequest(_)
        )
    }
}
