//! `PacificaAuthenticatedAdapter` — authenticated read-only adapter for Pacifica.

//!

//! Wraps `PacificaReadOnlyAdapter` and adds authenticated GET endpoints

//! (account info, builder program status). Does NOT submit orders. Iron Law §1.

//!

//! Auth scheme: simple `X-API-Key` header (no signing scheme needed — Pacifica

//! authenticated REST does not require EIP-191 or ed25519; that signing is only

//! used by `bot-nav` for on-chain NAV report submission). Credentials are loaded

//! from environment variables only — no config file, no keyring.

use std::sync::Arc;

use async_trait::async_trait;

use reqwest::Client;

use serde::{Deserialize, Serialize};

use crate::pacifica::{PacificaReadOnlyAdapter, PACIFICA_REST_URL};

use crate::venue::{
    AdapterError, FillReport, OrderIntent, PositionView, VenueAdapter, VenueSnapshot,
};

use bot_types::Venue;

// ── Auth credential env var names ─────────────────────────────────────────────

pub const ENV_API_KEY: &str = "PACIFICA_API_KEY";

pub const ENV_BUILDER_CODE: &str = "PACIFICA_BUILDER_CODE";

// ── Response shapes ───────────────────────────────────────────────────────────

/// Summary of the authenticated account state.
///
/// Fields map to the Pacifica `/account` authenticated response where known.
///
/// Unknown fields are preserved in `raw`. If the real API returns different
/// field names, adapt the `From<ApiAccountResponse>` impl below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub account: String,

    pub balance_usd: f64,

    pub margin_available_usd: f64,

    pub margin_locked_usd: f64,

    pub open_positions_count: u32,

    /// Full raw JSON preserved for forward-compat with unknown fields.

    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// Builder program status returned by the Pacifica builder endpoint.

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct BuilderStatus {
    pub builder_code: String,

    pub registered: bool,

    pub fee_tier: String,

    pub rebate_accrued_usd: f64,

    /// Unix ms of registration, if known.
    pub since: Option<i64>,

    /// Full raw JSON preserved for forward-compat with unknown fields.

    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

// ── Internal API response wrappers ────────────────────────────────────────────

/// Generic Pacifica API envelope (matches the pattern used in `bot-venues`).

#[derive(Debug, Deserialize)]

struct ApiEnvelope<T> {
    success: bool,

    data: Option<T>,

    error: Option<String>,
}

/// Raw account data from Pacifica (field names adapted from the public
/// `/account?account=` shape used in `PacificaRest::get_balance`).
#[derive(Debug, Deserialize)]
struct RawAccountData {
    #[serde(default)]
    account: String,

    #[serde(alias = "account_equity", default)]
    account_equity: String,

    #[serde(alias = "available_to_spend", default)]
    available_to_spend: String,

    #[serde(alias = "total_margin_used", default)]
    total_margin_used: String,

    #[serde(alias = "orders_count", default)]
    orders_count: Option<u32>,
}

/// Raw builder data from Pacifica.

#[derive(Debug, Deserialize)]

struct RawBuilderData {
    #[serde(alias = "builder_code", default)]
    builder_code: String,

    #[serde(default)]
    registered: bool,

    #[serde(alias = "fee_tier", default)]
    fee_tier: Option<String>,

    #[serde(alias = "rebate_accrued", default)]
    rebate_accrued: Option<String>,

    #[serde(alias = "since", default)]
    since: Option<i64>,
}

// ── Adapter struct ────────────────────────────────────────────────────────────

/// Authenticated read-only Pacifica adapter.
///
/// Delegates all `VenueAdapter` methods to the inner `PacificaReadOnlyAdapter`.
/// Additionally exposes `fetch_account_info` and `fetch_builder_status` which
/// require API key authentication.
///
/// The API key is **never** emitted in `Debug`, logs, or error messages.
pub struct PacificaAuthenticatedAdapter {
    inner: PacificaReadOnlyAdapter,

    client: Client,

    base_url: String,

    api_key: Arc<str>,

    builder_code: Arc<str>,
}

impl PacificaAuthenticatedAdapter {
    /// Construct with explicit credentials (for tests / overrides).
    ///
    /// Fails with `AdapterError::Parse` if either credential is empty.
    pub fn new(api_key: String, builder_code: String) -> Result<Self, AdapterError> {
        Self::new_with_url(api_key, builder_code, PACIFICA_REST_URL.to_string())
    }

    /// Construct against a custom base URL (useful for integration tests
    /// that point at a mock server).
    pub fn new_with_url(
        api_key: String,

        builder_code: String,

        base_url: String,
    ) -> Result<Self, AdapterError> {
        if api_key.is_empty() {
            return Err(AdapterError::Parse(
                "PACIFICA_API_KEY must not be empty".to_string(),
            ));
        }

        if builder_code.is_empty() {
            return Err(AdapterError::Parse(
                "PACIFICA_BUILDER_CODE must not be empty".to_string(),
            ));
        }

        Ok(Self {
            inner: PacificaReadOnlyAdapter::new(base_url.clone()),

            client: Client::new(),

            base_url: base_url.trim_end_matches('/').to_string(),

            api_key: Arc::from(api_key.as_str()),

            builder_code: Arc::from(builder_code.as_str()),
        })
    }

    /// Construct from environment variables `PACIFICA_API_KEY` and
    /// `PACIFICA_BUILDER_CODE`.
    ///
    /// Fails with a descriptive error if either variable is missing or empty.
    pub fn from_env() -> Result<Self, AdapterError> {
        let api_key = std::env::var(ENV_API_KEY).map_err(|_| {
            AdapterError::Parse(format!(
                "{ENV_API_KEY} is not set."
            ))
        })?;

        let builder_code = std::env::var(ENV_BUILDER_CODE).map_err(|_| {
            AdapterError::Parse(format!(
                "{ENV_BUILDER_CODE} is not set."
            ))
        })?;

        Self::new(api_key, builder_code)
    }

    /// The builder code (public identifier — safe to log).
    pub fn builder_code(&self) -> &str {
        &self.builder_code
    }

    /// Fetch the authenticated account summary.
    ///
    /// Issues `GET {base}/account?account=<builder_code>` with the API key header.
    /// Falls back to `/account/info` and `/user` if the primary path returns 404.
    pub async fn fetch_account_info(&self) -> Result<AccountInfo, AdapterError> {
        // Try the primary path first (same shape as PacificaRest::get_balance).

        let primary = format!("{}/account?account={}", self.base_url, self.builder_code);

        let fallbacks = [
            format!("{}/account/info", self.base_url),
            format!("{}/user", self.base_url),
        ];

        let mut last_err: Option<AdapterError> = None;

        for url in std::iter::once(primary).chain(fallbacks) {
            match self.get_account_from_url(&url).await {
                Ok(info) => return Ok(info),

                Err(AdapterError::SymbolNotFound(_)) => {
                    // 404 — try next endpoint

                    last_err = Some(AdapterError::SymbolNotFound(url));

                    continue;
                }

                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            AdapterError::Network("fetch_account_info: all endpoint paths returned 404".to_string())
        }))
    }

    async fn get_account_from_url(&self, url: &str) -> Result<AccountInfo, AdapterError> {
        let resp = self
            .client
            .get(url)
            .header("X-API-Key", self.api_key.as_ref())
            .send()
            .await
            .map_err(|e| AdapterError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AdapterError::SymbolNotFound(url.to_string()));
        }

        if !resp.status().is_success() {
            return Err(AdapterError::Network(format!(
                "pacifica /account: HTTP {}",
                resp.status()
            )));
        }

        let raw_val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::Parse(e.to_string()))?;

        // Try the standard envelope first.

        if let Ok(env) = serde_json::from_value::<ApiEnvelope<RawAccountData>>(raw_val.clone()) {
            if env.success {
                if let Some(d) = env.data {
                    return Ok(AccountInfo {
                        account: if d.account.is_empty() {
                            self.builder_code.to_string()
                        } else {
                            d.account
                        },

                        balance_usd: d.account_equity.parse().unwrap_or(0.0),

                        margin_available_usd: d.available_to_spend.parse().unwrap_or(0.0),

                        margin_locked_usd: d.total_margin_used.parse().unwrap_or(0.0),

                        open_positions_count: d.orders_count.unwrap_or(0),

                        raw: Some(raw_val),
                    });
                }
            }

            if !env.success {
                return Err(AdapterError::Network(format!(
                    "pacifica /account API error: {}",
                    env.error.unwrap_or_default()
                )));
            }
        }

        // Fallback: treat the raw value as the account object directly.

        Ok(AccountInfo {
            account: raw_val
                .get("account")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| self.builder_code.as_ref())
                .to_string(),

            balance_usd: raw_val
                .get("balance_usd")
                .or_else(|| raw_val.get("account_equity"))
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),

            margin_available_usd: raw_val
                .get("margin_available_usd")
                .or_else(|| raw_val.get("available_to_spend"))
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),

            margin_locked_usd: raw_val
                .get("margin_locked_usd")
                .or_else(|| raw_val.get("total_margin_used"))
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),

            open_positions_count: raw_val
                .get("open_positions_count")
                .or_else(|| raw_val.get("orders_count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,

            raw: Some(raw_val),
        })
    }

    /// Fetch the builder program status.
    ///
    /// Issues `GET {base}/builder/program?builder_code=<code>` with the API key header.
    pub async fn fetch_builder_status(&self) -> Result<BuilderStatus, AdapterError> {
        let url = format!(
            "{}/builder/program?builder_code={}",
            self.base_url, self.builder_code
        );

        let resp = self
            .client
            .get(&url)
            .header("X-API-Key", self.api_key.as_ref())
            .send()
            .await
            .map_err(|e| AdapterError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(AdapterError::Network(format!(
                "pacifica /builder/program: HTTP {}",
                resp.status()
            )));
        }

        let raw_val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::Parse(e.to_string()))?;

        // Try standard envelope.

        if let Ok(env) = serde_json::from_value::<ApiEnvelope<RawBuilderData>>(raw_val.clone()) {
            if env.success {
                if let Some(d) = env.data {
                    return Ok(BuilderStatus {
                        builder_code: if d.builder_code.is_empty() {
                            self.builder_code.to_string()
                        } else {
                            d.builder_code
                        },

                        registered: d.registered,

                        fee_tier: d.fee_tier.unwrap_or_else(|| "unknown".to_string()),

                        rebate_accrued_usd: d
                            .rebate_accrued
                            .as_deref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0),

                        since: d.since,

                        raw: Some(raw_val),
                    });
                }
            }

            if !env.success {
                return Err(AdapterError::Network(format!(
                    "pacifica /builder/program API error: {}",
                    env.error.unwrap_or_default()
                )));
            }
        }

        // Fallback: parse the raw value directly.

        Ok(BuilderStatus {
            builder_code: raw_val
                .get("builder_code")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| self.builder_code.as_ref())
                .to_string(),

            registered: raw_val
                .get("registered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),

            fee_tier: raw_val
                .get("fee_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),

            rebate_accrued_usd: raw_val
                .get("rebate_accrued_usd")
                .or_else(|| raw_val.get("rebate_accrued"))
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),

            since: raw_val.get("since").and_then(|v| v.as_i64()),

            raw: Some(raw_val),
        })
    }
}

// ── Redacted Debug ────────────────────────────────────────────────────────────

/// API key MUST never appear in debug output.
impl std::fmt::Debug for PacificaAuthenticatedAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacificaAuthenticatedAdapter")
            .field("api_key", &"<redacted>")
            .field("builder_code", &self.builder_code.as_ref())
            .finish()
    }
}

// ── VenueAdapter delegation ───────────────────────────────────────────────────

/// Delegate all `VenueAdapter` methods to the inner read-only adapter so the
/// tick engine can use `PacificaAuthenticatedAdapter` wherever
/// `PacificaReadOnlyAdapter` is accepted.
#[async_trait]
impl VenueAdapter for PacificaAuthenticatedAdapter {
    fn venue(&self) -> Venue {
        self.inner.venue()
    }

    async fn fetch_snapshot(&self, symbol: &str) -> Result<VenueSnapshot, AdapterError> {
        self.inner.fetch_snapshot(symbol).await
    }

    async fn list_symbols(&self) -> Result<Vec<String>, AdapterError> {
        self.inner.list_symbols().await
    }

    async fn fetch_position(&self, symbol: &str) -> Result<Option<PositionView>, AdapterError> {
        self.inner.fetch_position(symbol).await
    }

    async fn submit_dryrun(&self, order: &OrderIntent) -> Result<FillReport, AdapterError> {
        self.inner.submit_dryrun(order).await
    }
}
