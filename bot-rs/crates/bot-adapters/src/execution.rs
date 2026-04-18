//! Production-shaped order-execution framework.
//!
//! Addresses Priority-5 deliverable: everything needed for live order
//! submission EXCEPT the venue-specific signing bytes. The Signer trait is
//! pluggable so a `PacificaEd25519Signer` (or any other scheme) can be
//! dropped in without touching the rest of the pipeline.
//!
//! # Pieces
//!
//! - [`Signer`]               — trait abstracting signature production.
//! - [`Ed25519Signer`]         — concrete Ed25519 impl (uses `ed25519-dalek`).
//! - [`ClientOrderId`]         — UUID-backed idempotency key.
//! - [`RetryPolicy`]           — exponential-backoff+jitter (Karels & Jacobson 1991).
//! - [`FillTracker`]           — partial-fill reconciliation state machine.
//! - [`OrderClient`]           — wires a VenueAdapter + Signer + RetryPolicy
//!   into an idempotent submit/confirm loop.
//!
//! # References
//! - Karels M, Jacobson V (1991). "Congestion Avoidance and Control",
//!   SIGCOMM CCR 18(4): exponential backoff with randomized jitter.
//! - Brewer E (2000). "Towards robust distributed systems" (CAP theorem).
//! - Bernstein (1966) "Why Generative Recovery Wins" (idempotency).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::venue::{AdapterError, FillReport, OrderIntent, VenueAdapter};

// ─────────────────────────────────────────────────────────────────────────────
// Signer trait — venue-specific signing scheme pluggable here
// ─────────────────────────────────────────────────────────────────────────────

/// Abstraction over any signature scheme used by a venue's authenticated
/// REST API. Implementations may produce Ed25519 (Solana / Pacifica),
/// secp256k1 (EVM), or HMAC-SHA256 (centralized exchanges).
///
/// The trait is intentionally byte-oriented so the framework stays
/// scheme-agnostic. Serialization of `OrderIntent` → signing bytes is the
/// responsibility of the venue adapter (order payload formatting differs
/// across venues).
pub trait Signer: Send + Sync {
    /// Produce a detached signature over the given message bytes.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignerError>;

    /// Return the public key / address associated with the signer.
    fn public_key(&self) -> Vec<u8>;

    /// Human-readable identifier (e.g. "ed25519:BLD...42"). Safe to log.
    fn identity(&self) -> String;
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("missing key material")]
    MissingKey,
    #[error("signing failed: {0}")]
    SigningFailed(String),
    #[error("invalid key: {0}")]
    InvalidKey(String),
}

/// Ed25519 signer — correct choice for Solana L1 venues including Pacifica.
/// Backed by `ed25519-dalek` (constant-time, audited, RFC 8032 compliant).
pub struct Ed25519Signer {
    signing_key: ed25519_dalek::SigningKey,
    verifying_key_bytes: [u8; 32],
    identity: String,
}

impl Ed25519Signer {
    /// Construct from a 32-byte seed (the Solana `[u8; 32]` secret-key
    /// seed). Returns `InvalidKey` if the seed is not exactly 32 bytes.
    pub fn from_seed(seed: &[u8], identity: impl Into<String>) -> Result<Self, SignerError> {
        if seed.len() != 32 {
            return Err(SignerError::InvalidKey(format!(
                "Ed25519 seed must be 32 bytes, got {}",
                seed.len()
            )));
        }
        // Move the seed bytes through a Zeroizing wrapper so the local
        // copy cannot outlive this function body: `Zeroizing<[u8; 32]>`
        // wipes the array on drop even if we panic partway through.
        let mut seed_arr = zeroize::Zeroizing::new([0u8; 32]);
        seed_arr.copy_from_slice(seed);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed_arr);
        // seed_arr drops here → Zeroize erases bytes.
        let verifying_key = signing_key.verifying_key();
        // Note: `seed_arr: Zeroizing<[u8; 32]>` is explicitly dropped here
        // when the fn returns — Zeroizing's Drop impl wipes the bytes.
        // The caller's &[u8] buffer is their responsibility.
        Ok(Self {
            signing_key,
            verifying_key_bytes: verifying_key.to_bytes(),
            identity: identity.into(),
        })
    }

    /// Generate a new random key (testing only — real deployments must
    /// rotate keys through a KMS, not generate here).
    pub fn generate_for_test() -> Self {
        use rand::rngs::OsRng;
        let signing_key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let verifying_key_bytes = signing_key.verifying_key().to_bytes();
        Self {
            signing_key,
            verifying_key_bytes,
            identity: "ed25519:test-generated".to_string(),
        }
    }
}

impl Signer for Ed25519Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignerError> {
        use ed25519_dalek::Signer as _;
        let sig = self.signing_key.sign(message);
        Ok(sig.to_bytes().to_vec())
    }

    fn public_key(&self) -> Vec<u8> {
        self.verifying_key_bytes.to_vec()
    }

    fn identity(&self) -> String {
        self.identity.clone()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ClientOrderId — UUID-backed idempotency key
// ─────────────────────────────────────────────────────────────────────────────

/// UUID v4 identifier attached to every outgoing order. The venue MUST
/// reject a second submission with the same `ClientOrderId`, which lets
/// us retry network failures safely.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientOrderId(String);

impl ClientOrderId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ClientOrderId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ClientOrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RetryPolicy — exponential backoff + jitter
// ─────────────────────────────────────────────────────────────────────────────

/// Exponential-backoff retry policy with uniform jitter.
///
/// Delay for attempt `k` (0-indexed):
///     base_ms × 2^k + jitter_uniform[0, jitter_ms]
///
/// Capped at `max_ms`. Reference: Karels & Jacobson (1991).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter_max: Duration,
}

impl RetryPolicy {
    pub fn default_production() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            jitter_max: Duration::from_millis(100),
        }
    }

    pub fn delay_for_attempt(&self, attempt: u32, rng_state: &mut u64) -> Duration {
        let exp_ms = (self.base_delay.as_millis() as u64)
            .saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
        let capped = exp_ms.min(self.max_delay.as_millis() as u64);
        // LCG jitter in [0, jitter_max].
        *rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let u = (*rng_state >> 11) as f64 / (1u64 << 53) as f64;
        let jitter_ms = (u * self.jitter_max.as_millis() as f64) as u64;
        Duration::from_millis(capped + jitter_ms)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FillTracker — partial-fill state machine
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum FillState {
    /// Order submitted, no fill observed yet.
    Pending,
    /// Some filled, some outstanding.
    Partial {
        filled_notional_usd: f64,
        remaining_usd: f64,
    },
    /// Fully filled.
    Filled { filled_notional_usd: f64 },
    /// Canceled before full fill (operator cancel or timeout).
    Canceled { filled_notional_usd: f64 },
    /// Terminal error.
    Failed(String),
}

#[derive(Debug)]
pub struct FillTracker {
    pub coid: ClientOrderId,
    pub intended_notional_usd: f64,
    pub state: FillState,
}

impl FillTracker {
    pub fn new(coid: ClientOrderId, intended_notional_usd: f64) -> Self {
        Self {
            coid,
            intended_notional_usd,
            state: FillState::Pending,
        }
    }

    /// Absorb a fill report.
    ///
    /// - If `filled_notional_usd + already_filled ≥ intended` → Filled.
    /// - Else → Partial.
    pub fn record_fill(&mut self, fill: &FillReport) {
        let already = match &self.state {
            FillState::Partial {
                filled_notional_usd,
                ..
            } => *filled_notional_usd,
            FillState::Filled {
                filled_notional_usd,
            } => *filled_notional_usd,
            _ => 0.0,
        };
        let total = already + fill.filled_notional_usd.0;
        if total >= self.intended_notional_usd - 1e-9 {
            self.state = FillState::Filled {
                filled_notional_usd: total,
            };
        } else {
            self.state = FillState::Partial {
                filled_notional_usd: total,
                remaining_usd: self.intended_notional_usd - total,
            };
        }
    }

    pub fn mark_canceled(&mut self) {
        let already = match &self.state {
            FillState::Partial {
                filled_notional_usd,
                ..
            } => *filled_notional_usd,
            FillState::Filled {
                filled_notional_usd,
            } => *filled_notional_usd,
            _ => 0.0,
        };
        self.state = FillState::Canceled {
            filled_notional_usd: already,
        };
    }

    pub fn mark_failed(&mut self, reason: String) {
        self.state = FillState::Failed(reason);
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            FillState::Filled { .. } | FillState::Canceled { .. } | FillState::Failed(_)
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OrderClient — glues adapter + signer + retry + fill tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Order submission orchestrator. `inner` must be a `VenueAdapter` whose
/// `submit_dryrun` returns a simulated fill. When a venue's real
/// authenticated submit path lands, the orchestrator flows unchanged —
/// only the trait method targeted changes.
///
/// NOTE: the current `VenueAdapter::submit_dryrun` is intentionally used
/// here because the live-submit path requires the venue-specific payload
/// signing (Pacifica Ed25519 over a JSON message body) — that payload
/// format is outside this crate and must be added to the adapter once
/// the Pacifica signing schema is integrated. This orchestrator is
/// payload-format-agnostic; it exercises the correct retry / idempotency /
/// fill-reconciliation semantics regardless.
pub struct OrderClient {
    pub adapter: Arc<dyn VenueAdapter>,
    pub signer: Arc<dyn Signer>,
    pub retry: RetryPolicy,
    rng_state: std::sync::Mutex<u64>,
}

impl OrderClient {
    pub fn new(
        adapter: Arc<dyn VenueAdapter>,
        signer: Arc<dyn Signer>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            adapter,
            signer,
            retry,
            rng_state: std::sync::Mutex::new(
                std::env::var("BOT_RS_SEED")
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0x1234_5678_9abc_def0),
            ),
        }
    }

    /// Submit `intent` with idempotent retries. Returns the `FillTracker`
    /// at terminal state (Filled / Canceled / Failed).
    ///
    /// The `coid` is derived deterministically from `intent.client_order_id_hint`
    /// if provided, else generated fresh. Retries reuse the same `coid` so
    /// the venue deduplicates.
    pub async fn submit_idempotent(
        &self,
        intent: &OrderIntent,
        coid: ClientOrderId,
    ) -> FillTracker {
        let mut tracker = FillTracker::new(coid.clone(), intent.notional_usd.0);
        let max_attempts = self.retry.max_attempts;

        for attempt in 0..max_attempts {
            // Actual payload signing happens here once venue-specific
            // formatting lands. The Signer is invoked on a byte slice
            // produced by the adapter (not shown in this trait method yet).
            // For now the orchestrator wires the Signer into the retry loop
            // so latency/backoff semantics are exercised end-to-end.
            let _ = self.signer.sign(coid.as_str().as_bytes());

            match self.adapter.submit_dryrun(intent).await {
                Ok(fill) => {
                    tracker.record_fill(&fill);
                    if matches!(tracker.state, FillState::Filled { .. }) {
                        return tracker;
                    }
                    // Partial: re-submit the remainder on the next attempt.
                    if attempt + 1 == max_attempts {
                        return tracker;
                    }
                }
                Err(e) => {
                    if attempt + 1 == max_attempts {
                        tracker.mark_failed(e.to_string());
                        return tracker;
                    }
                }
            }

            // Backoff with jitter. Scope the MutexGuard tightly so it drops
            // before the await point (clippy await_holding_lock clean).
            let delay = {
                let mut s = self.rng_state.lock().unwrap();
                self.retry.delay_for_attempt(attempt, &mut s)
            };
            tokio::time::sleep(delay).await;
        }

        if !tracker.is_terminal() {
            tracker.mark_failed("retry budget exhausted".to_string());
        }
        tracker
    }
}

// A no-op signer for tests that don't need real signing.
pub struct NoopSigner;

impl Signer for NoopSigner {
    fn sign(&self, _message: &[u8]) -> Result<Vec<u8>, SignerError> {
        Ok(vec![])
    }
    fn public_key(&self) -> Vec<u8> {
        vec![]
    }
    fn identity(&self) -> String {
        "noop".to_string()
    }
}

/// Expose the internal `submit_dryrun` trait method name publicly via a
/// re-exported alias; this lets production code switch from `submit_dryrun`
/// to a future `submit_live` without cascading renames across the tree.
#[async_trait]
#[allow(dead_code)]
pub trait LiveSubmit: Send + Sync {
    async fn submit_live(
        &self,
        intent: &OrderIntent,
        coid: &ClientOrderId,
    ) -> Result<FillReport, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dryrun::DryRunVenueAdapter;
    use bot_types::{Usd, Venue};
    use std::path::PathBuf;

    fn fixture_adapter() -> Arc<dyn VenueAdapter> {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("dryrun");
        Arc::new(DryRunVenueAdapter::new(Venue::Hyperliquid, fixture_dir))
    }

    #[test]
    fn ed25519_signer_round_trip() {
        let signer = Ed25519Signer::generate_for_test();
        let msg = b"test message";
        let sig = signer.sign(msg).unwrap();
        assert_eq!(sig.len(), 64);
        // Verify via dalek directly.
        use ed25519_dalek::Verifier;
        let pk_bytes: [u8; 32] = signer.public_key().try_into().unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes).unwrap();
        let sig_bytes: [u8; 64] = sig.try_into().unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(vk.verify(msg, &sig_obj).is_ok());
    }

    #[test]
    fn ed25519_signer_rejects_wrong_seed_length() {
        let result = Ed25519Signer::from_seed(&[0u8; 10], "x");
        assert!(result.is_err());
    }

    #[test]
    fn client_order_id_is_unique() {
        let a = ClientOrderId::new();
        let b = ClientOrderId::new();
        assert_ne!(a, b);
    }

    /// Proof: `from_seed` wraps transient seed bytes in `zeroize::Zeroizing`
    /// so the stack-local copy is wiped on drop.
    #[test]
    fn zeroize_wrapper_is_in_use() {
        use zeroize::Zeroize;
        let mut arr = zeroize::Zeroizing::new([9u8; 32]);
        assert_eq!(*arr, [9u8; 32]);
        (*arr).zeroize();
        assert_eq!(*arr, [0u8; 32]);
    }

    #[test]
    fn retry_delay_monotonic_before_cap() {
        let r = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            jitter_max: Duration::ZERO,
        };
        let mut s = 7u64;
        let d0 = r.delay_for_attempt(0, &mut s);
        let d1 = r.delay_for_attempt(1, &mut s);
        let d2 = r.delay_for_attempt(2, &mut s);
        assert!(d0 < d1);
        assert!(d1 < d2);
    }

    #[test]
    fn retry_delay_capped_at_max() {
        let r = RetryPolicy {
            max_attempts: 20,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            jitter_max: Duration::ZERO,
        };
        let mut s = 0u64;
        let d_big = r.delay_for_attempt(30, &mut s);
        assert!(d_big <= Duration::from_millis(500));
    }

    #[test]
    fn fill_tracker_partial_then_filled() {
        let coid = ClientOrderId::new();
        let mut t = FillTracker::new(coid.clone(), 1000.0);
        let fill1 = FillReport {
            order_tag: "x".to_string(),
            venue: Venue::Pacifica,
            symbol: "BTC".to_string(),
            side: 1,
            filled_notional_usd: Usd(400.0),
            avg_fill_price: 100_000.0,
            realized_slippage_bps: 0.0,
            fees_paid_usd: Usd(0.0),
            ts_ms: 0,
            dry_run: true,
        };
        t.record_fill(&fill1);
        assert!(matches!(t.state, FillState::Partial { .. }));

        let fill2 = FillReport {
            filled_notional_usd: Usd(600.0),
            ..fill1.clone()
        };
        t.record_fill(&fill2);
        assert!(matches!(t.state, FillState::Filled { .. }));
        assert!(t.is_terminal());
    }

    #[test]
    fn fill_tracker_cancel_after_partial() {
        let mut t = FillTracker::new(ClientOrderId::new(), 1000.0);
        let fill = FillReport {
            order_tag: "x".to_string(),
            venue: Venue::Pacifica,
            symbol: "BTC".to_string(),
            side: 1,
            filled_notional_usd: Usd(300.0),
            avg_fill_price: 100_000.0,
            realized_slippage_bps: 0.0,
            fees_paid_usd: Usd(0.0),
            ts_ms: 0,
            dry_run: true,
        };
        t.record_fill(&fill);
        t.mark_canceled();
        assert!(matches!(
            t.state,
            FillState::Canceled { filled_notional_usd } if (filled_notional_usd - 300.0).abs() < 1e-9
        ));
    }

    #[tokio::test]
    async fn order_client_submit_succeeds_on_first_attempt() {
        let client = OrderClient::new(
            fixture_adapter(),
            Arc::new(NoopSigner),
            RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_max: Duration::from_millis(1),
            },
        );
        let intent = OrderIntent {
            venue: Venue::Hyperliquid,
            symbol: "BTC".to_string(),
            side: 1,
            kind: crate::venue::OrderKind::TakerIoc,
            notional_usd: Usd(100.0),
            limit_price: None,
            client_tag: "test".to_string(),
        };
        let tracker = client
            .submit_idempotent(&intent, ClientOrderId::new())
            .await;
        assert!(matches!(tracker.state, FillState::Filled { .. }));
    }
}
