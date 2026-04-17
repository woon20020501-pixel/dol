//! Pacifica order-signing payload construction — exact parity with the
//! official Pacifica Python SDK (`pacifica-fi/python-sdk`).
//!
//! # Spec (as of main @ 2026-04-17, file `common/utils.py`)
//!
//! 1. Build header: `{"type", "timestamp", "expiry_window"}`.
//! 2. Build payload: per-endpoint fields (e.g. `{symbol, amount, side, ...}`).
//! 3. Form `data = {**header, "data": payload}`.
//! 4. **Sort every dict's keys alphabetically, recursively** (`sort_json_keys`).
//! 5. Serialize to compact JSON with `separators=(",", ":")` — NO whitespace.
//! 6. Encode UTF-8 → bytes.
//! 7. Ed25519-sign with Solana keypair.
//! 8. Base58-encode the 64-byte signature.
//!
//! # REST body
//!
//! The HTTP POST body is `{account, signature, timestamp, expiry_window, **payload}`
//! where `account` = base58 pubkey, `signature` = base58 sig, and payload
//! fields are flattened at the top level (NOT nested under "data").
//!
//! # Endpoints
//!
//! - Limit order  : `POST /api/v1/orders/create`          type = `"create_order"`
//! - Market order : `POST /api/v1/orders/create_market`   type = `"create_market_order"`
//! - Cancel       : `POST /api/v1/orders/cancel`          (see cancel_order.py)
//!
//! # References
//!
//! - Pacifica Python SDK: <https://github.com/pacifica-fi/python-sdk>
//! - `common/utils.py` — `sign_message`, `prepare_message`, `sort_json_keys`
//! - `rest/create_market_order.py` — market order example
//! - `rest/create_limit_order.py` — limit order example
//! - RFC 8032: EdDSA/Ed25519

use serde::Serialize;
use serde_json::{Map, Value};

use crate::execution::{Signer, SignerError};

/// Pacifica mainnet REST base URL.
pub const MAINNET_REST_URL: &str = "https://api.pacifica.fi/api/v1";
/// Pacifica testnet REST base URL.
pub const TESTNET_REST_URL: &str = "https://test-api.pacifica.fi/api/v1";

/// Message header — the three fields Pacifica requires before any payload.
///
/// Field order in JSON output does NOT matter here because we sort keys
/// alphabetically before serialization (parity with Python SDK).
#[derive(Debug, Clone, Serialize)]
pub struct PacificaHeader {
    /// Message type — e.g. `"create_order"` or `"create_market_order"`.
    pub r#type: String,
    /// Unix milliseconds (matches Python `int(time.time() * 1000)`).
    pub timestamp: i64,
    /// Expiry window in ms (default 5000 in SDK).
    pub expiry_window: i64,
}

impl PacificaHeader {
    pub fn new(type_: impl Into<String>, timestamp_ms: i64, expiry_window_ms: i64) -> Self {
        Self {
            r#type: type_.into(),
            timestamp: timestamp_ms,
            expiry_window: expiry_window_ms,
        }
    }
}

/// Limit order payload (matches `rest/create_limit_order.py`).
#[derive(Debug, Clone, Serialize)]
pub struct LimitOrderPayload {
    pub symbol: String,
    /// Stringified price (SDK passes `str(100_000)`, NOT a number).
    pub price: String,
    pub reduce_only: bool,
    /// Stringified amount (e.g. `"0.1"`).
    pub amount: String,
    /// `"bid"` for buy, `"ask"` for sell.
    pub side: String,
    /// Time-in-force: `"GTC"` | `"IOC"` | `"FOK"` (see SDK).
    pub tif: String,
    /// Client order ID — UUID v4 string per SDK (`str(uuid.uuid4())`).
    pub client_order_id: String,
}

/// Market order payload (matches `rest/create_market_order.py`).
#[derive(Debug, Clone, Serialize)]
pub struct MarketOrderPayload {
    pub symbol: String,
    pub reduce_only: bool,
    pub amount: String,
    pub side: String,
    /// Stringified max slippage percent (e.g. `"0.5"`).
    pub slippage_percent: String,
    pub client_order_id: String,
}

/// Cancel order payload.
#[derive(Debug, Clone, Serialize)]
pub struct CancelOrderPayload {
    pub symbol: String,
    /// Either `order_id` OR `client_order_id` is provided; the other is None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Message bytes construction — byte-exact Python parity
// ─────────────────────────────────────────────────────────────────────────────

/// Recursively sort dict keys alphabetically. Matches Python
/// `sort_json_keys` in `common/utils.py` (uses `sorted(value.keys())`).
///
/// The result is a `serde_json::Value` tree where every `Object` child has
/// been rebuilt via `BTreeMap` (alphabetical by key).
pub fn sort_json_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            // Use BTreeMap to produce sorted key ordering.
            let mut sorted: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k, sort_json_keys(v));
            }
            // Re-serialize into a serde_json::Map preserving the BTreeMap order.
            let mut out_map = Map::new();
            for (k, v) in sorted {
                out_map.insert(k, v);
            }
            Value::Object(out_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_json_keys).collect()),
        other => other,
    }
}

/// Build the canonical signing message bytes for a header+payload pair,
/// byte-exact with Python `prepare_message`.
///
/// Algorithm:
/// 1. Serialize `header` to JSON Object (via serde).
/// 2. Serialize `payload` to JSON Value.
/// 3. `data = {**header_map, "data": payload}`.
/// 4. `sort_json_keys(data)` — recursively sort.
/// 5. `serde_json::to_string` with compact formatter (default separators
///    are `","` and `":"` — already compact in serde).
///
/// NOTE: serde_json's default output already has no whitespace between
/// tokens (matches Python `separators=(",", ":")`). We rely on that.
///
/// Validates presence of the three header fields per the Python
/// ValueError guard.
pub fn prepare_message<H, P>(header: &H, payload: &P) -> Result<String, SignerError>
where
    H: Serialize,
    P: Serialize,
{
    // Step 1: serialize header.
    let header_v = serde_json::to_value(header)
        .map_err(|e| SignerError::SigningFailed(format!("header serialize: {e}")))?;
    let header_map = match header_v {
        Value::Object(m) => m,
        _ => {
            return Err(SignerError::SigningFailed(
                "header must serialize to a JSON object".into(),
            ));
        }
    };
    // Mirror the Python ValueError: require {type, timestamp, expiry_window}.
    for required in ["type", "timestamp", "expiry_window"] {
        if !header_map.contains_key(required) {
            return Err(SignerError::SigningFailed(format!(
                "header missing required field `{required}`"
            )));
        }
    }
    // Step 2: serialize payload.
    let payload_v = serde_json::to_value(payload)
        .map_err(|e| SignerError::SigningFailed(format!("payload serialize: {e}")))?;
    // Step 3: merge — `{**header, "data": payload}`.
    let mut data_map = header_map;
    data_map.insert("data".to_string(), payload_v);
    let data = Value::Object(data_map);
    // Step 4: sort keys.
    let sorted = sort_json_keys(data);
    // Step 5: compact JSON (serde_json default is already compact).
    serde_json::to_string(&sorted)
        .map_err(|e| SignerError::SigningFailed(format!("serialize sorted data: {e}")))
}

/// Sign a header+payload via any [`Signer`]. Returns
/// `(message_string, base58_signature)` exactly as the Python SDK does.
pub fn sign_message<H, P>(
    header: &H,
    payload: &P,
    signer: &dyn Signer,
) -> Result<(String, String), SignerError>
where
    H: Serialize,
    P: Serialize,
{
    let message = prepare_message(header, payload)?;
    let sig_bytes = signer.sign(message.as_bytes())?;
    Ok((message, bs58::encode(sig_bytes).into_string()))
}

/// Build the final HTTP request body: flatten `{account, signature,
/// timestamp, expiry_window, **payload}` at the top level (NOT nested).
///
/// `account` = base58 pubkey of the signer.
/// `signature` = base58 output from [`sign_message`].
pub fn build_request_body<P: Serialize>(
    account_base58: &str,
    signature_base58: &str,
    header: &PacificaHeader,
    payload: &P,
) -> Result<Value, SignerError> {
    let payload_v = serde_json::to_value(payload)
        .map_err(|e| SignerError::SigningFailed(format!("payload serialize: {e}")))?;
    let payload_map = match payload_v {
        Value::Object(m) => m,
        _ => {
            return Err(SignerError::SigningFailed(
                "payload must serialize to a JSON object".into(),
            ));
        }
    };
    let mut body = Map::new();
    body.insert(
        "account".to_string(),
        Value::String(account_base58.to_string()),
    );
    body.insert(
        "signature".to_string(),
        Value::String(signature_base58.to_string()),
    );
    body.insert(
        "timestamp".to_string(),
        Value::Number(header.timestamp.into()),
    );
    body.insert(
        "expiry_window".to_string(),
        Value::Number(header.expiry_window.into()),
    );
    for (k, v) in payload_map {
        body.insert(k, v);
    }
    Ok(Value::Object(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::Ed25519Signer;

    #[test]
    fn sort_json_keys_alphabetizes_recursively() {
        let v: Value =
            serde_json::from_str(r#"{"b":1,"a":{"z":2,"y":1},"c":[{"b":2,"a":1}]}"#).unwrap();
        let sorted = sort_json_keys(v);
        let out = serde_json::to_string(&sorted).unwrap();
        // Keys now alphabetical at every level.
        assert_eq!(out, r#"{"a":{"y":1,"z":2},"b":1,"c":[{"a":1,"b":2}]}"#);
    }

    #[test]
    fn prepare_message_rejects_missing_header_fields() {
        #[derive(Serialize)]
        struct BadHeader {
            r#type: String,
            timestamp: i64,
            // missing expiry_window
        }
        let bad = BadHeader {
            r#type: "x".into(),
            timestamp: 0,
        };
        let payload: Value = serde_json::json!({});
        let result = prepare_message(&bad, &payload);
        assert!(result.is_err());
    }

    #[test]
    fn prepare_message_matches_python_shape_for_limit_order() {
        // This is the exact payload from `rest/create_limit_order.py` with
        // the UUID replaced by a fixed string for determinism.
        let header = PacificaHeader::new("create_order", 1_700_000_000_000, 5_000);
        let payload = LimitOrderPayload {
            symbol: "BTC".into(),
            price: "100000".into(),
            reduce_only: false,
            amount: "0.1".into(),
            side: "bid".into(),
            tif: "GTC".into(),
            client_order_id: "abc-123".into(),
        };
        let message = prepare_message(&header, &payload).unwrap();
        // Expected structure: {"data":<sorted_payload>,"expiry_window":5000,"timestamp":...,"type":"create_order"}
        // Where sorted_payload has its keys alphabetical.
        let expected = concat!(
            r#"{"data":{"#,
            r#""amount":"0.1","client_order_id":"abc-123","price":"100000","reduce_only":false,"side":"bid","symbol":"BTC","tif":"GTC""#,
            r#"},"expiry_window":5000,"timestamp":1700000000000,"type":"create_order"}"#
        );
        assert_eq!(message, expected);
    }

    #[test]
    fn prepare_message_matches_python_shape_for_market_order() {
        let header = PacificaHeader::new("create_market_order", 1_700_000_000_000, 5_000);
        let payload = MarketOrderPayload {
            symbol: "BTC".into(),
            reduce_only: false,
            amount: "0.1".into(),
            side: "bid".into(),
            slippage_percent: "0.5".into(),
            client_order_id: "abc-123".into(),
        };
        let message = prepare_message(&header, &payload).unwrap();
        let expected = concat!(
            r#"{"data":{"#,
            r#""amount":"0.1","client_order_id":"abc-123","reduce_only":false,"side":"bid","slippage_percent":"0.5","symbol":"BTC""#,
            r#"},"expiry_window":5000,"timestamp":1700000000000,"type":"create_market_order"}"#
        );
        assert_eq!(message, expected);
    }

    #[test]
    fn sign_message_produces_64_byte_base58_signature() {
        let signer = Ed25519Signer::generate_for_test();
        let header = PacificaHeader::new("create_order", 1, 5000);
        let payload = MarketOrderPayload {
            symbol: "BTC".into(),
            reduce_only: false,
            amount: "0.1".into(),
            side: "bid".into(),
            slippage_percent: "0.5".into(),
            client_order_id: "x".into(),
        };
        let (msg, sig_b58) = sign_message(&header, &payload, &signer).unwrap();
        // Decode base58 back to bytes and verify length + Ed25519 signature.
        let sig_bytes = bs58::decode(&sig_b58).into_vec().unwrap();
        assert_eq!(sig_bytes.len(), 64);
        use ed25519_dalek::Verifier;
        let pk: [u8; 32] = signer.public_key().try_into().unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).unwrap();
        let sig: [u8; 64] = sig_bytes.try_into().unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify(msg.as_bytes(), &sig_obj).is_ok());
    }

    #[test]
    fn build_request_body_flattens_payload() {
        let header = PacificaHeader::new("create_order", 42, 5000);
        let payload = MarketOrderPayload {
            symbol: "ETH".into(),
            reduce_only: true,
            amount: "1".into(),
            side: "ask".into(),
            slippage_percent: "0.1".into(),
            client_order_id: "cid".into(),
        };
        let body = build_request_body("PUBKEY_B58", "SIG_B58", &header, &payload).unwrap();
        assert_eq!(body["account"], "PUBKEY_B58");
        assert_eq!(body["signature"], "SIG_B58");
        assert_eq!(body["timestamp"], 42);
        assert_eq!(body["expiry_window"], 5000);
        // Payload fields flattened at top level, not nested.
        assert_eq!(body["symbol"], "ETH");
        assert_eq!(body["side"], "ask");
        assert_eq!(body["amount"], "1");
        // "data" key must NOT be present in the HTTP body (only in signed msg).
        assert!(body.get("data").is_none());
    }

    #[test]
    fn sort_matches_python_nested_case() {
        // Verifies recursion — nested object with its own keys out of order.
        let v: Value = serde_json::json!({
            "z": {"c": 3, "a": 1, "b": 2},
            "a": [1, 2, 3],
            "m": {"nested": {"y": 1, "x": 2}}
        });
        let sorted = sort_json_keys(v);
        let out = serde_json::to_string(&sorted).unwrap();
        assert_eq!(
            out,
            r#"{"a":[1,2,3],"m":{"nested":{"x":2,"y":1}},"z":{"a":1,"b":2,"c":3}}"#
        );
    }
}
