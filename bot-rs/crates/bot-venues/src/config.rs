use crate::error::VenueError;

/// Pacifica venue configuration. Loaded from env vars.
#[derive(Debug, Clone)]
pub struct PacificaConfig {
    pub ws_url: String,
    pub rest_url: String,
    pub account: String,
    pub symbol: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

/// Lighter venue configuration. Loaded from env vars.
#[derive(Debug, Clone)]
pub struct LighterConfig {
    pub ws_url: String,
    pub rest_url: String,
    pub account_index: Option<String>,
    pub l1_address: Option<String>,
    pub symbol: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

// Default mainnet URLs
const PACIFICA_WS_DEFAULT: &str = "wss://ws.pacifica.fi/ws";
const PACIFICA_REST_DEFAULT: &str = "https://api.pacifica.fi/api/v1";
const LIGHTER_WS_DEFAULT: &str = "wss://mainnet.zklighter.elliot.ai/stream";
const LIGHTER_REST_DEFAULT: &str = "https://mainnet.zklighter.elliot.ai/api/v1";
const DEFAULT_SYMBOL: &str = "USDJPY";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

impl PacificaConfig {
    /// Load from environment variables. Call `dotenvy::dotenv().ok()`
    /// before this to pick up `.env` files.
    pub fn from_env() -> Result<Self, VenueError> {
        let account = env_opt("PACIFICA_ACCOUNT")
            .ok_or_else(|| VenueError::ConfigMissing("PACIFICA_ACCOUNT".into()))?;

        Ok(Self {
            ws_url: env_or("PACIFICA_WS_URL", PACIFICA_WS_DEFAULT),
            rest_url: env_or("PACIFICA_REST_URL", PACIFICA_REST_DEFAULT),
            account,
            symbol: env_or("SYMBOL", DEFAULT_SYMBOL),
            api_key: env_opt("PACIFICA_API_KEY"),
            api_secret: env_opt("PACIFICA_API_SECRET"),
        })
    }
}

impl LighterConfig {
    /// Load from environment variables.
    ///
    /// Account resolution priority:
    /// 1. `LIGHTER_ACCOUNT_INDEX` if set → use directly
    /// 2. `LIGHTER_L1_ADDRESS` if set → REST lookup at runtime
    /// 3. Neither → error
    pub fn from_env() -> Result<Self, VenueError> {
        let account_index = env_opt("LIGHTER_ACCOUNT_INDEX");
        let l1_address = env_opt("LIGHTER_L1_ADDRESS");

        if account_index.is_none() && l1_address.is_none() {
            return Err(VenueError::ConfigMissing(
                "LIGHTER_ACCOUNT_INDEX or LIGHTER_L1_ADDRESS".into(),
            ));
        }

        Ok(Self {
            ws_url: env_or("LIGHTER_WS_URL", LIGHTER_WS_DEFAULT),
            rest_url: env_or("LIGHTER_REST_URL", LIGHTER_REST_DEFAULT),
            account_index,
            l1_address,
            symbol: env_or("SYMBOL", DEFAULT_SYMBOL),
            api_key: env_opt("LIGHTER_API_KEY"),
            api_secret: env_opt("LIGHTER_API_SECRET"),
        })
    }
}
