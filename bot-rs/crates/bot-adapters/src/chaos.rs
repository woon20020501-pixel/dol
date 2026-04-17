//! Chaos / latency injection wrapper around any `VenueAdapter`.
//!
//! Used by the chaos-testing harness and by offline paper-trading
//! backtests where realistic adapter latency must be modeled.
//!
//! Two independent knobs per wrapped adapter:
//!
//! - **Latency profile** — constant, uniform-random, or fixed-sequence
//!   millisecond delay added to every `fetch_snapshot` call. Calibrated from
//!   real-world venue latency distributions:
//!
//!   - Pacifica mainnet REST: 200-400ms typical, 1200ms p99
//!   - Hyperliquid mainnet: 80-200ms typical, 500ms p99
//!   - Lighter mainnet: 100-300ms typical
//!   - Backpack mainnet: 60-180ms typical
//!
//!   Source: repeat-probe measurements across 2026-Q1; documented in
//!   `docs/latency-profile.md`.
//!
//! - **Failure profile** — deterministic error-ratio (e.g. 1 in N calls
//!   returns `AdapterError::Network`) OR a user-provided scripted sequence
//!   (`[Ok, Ok, Err, Ok, ...]`) for reproducible chaos tests.
//!
//! The wrapper is a decorator: any code that accepts
//! `Arc<dyn VenueAdapter>` can transparently use a ChaosAdapter by
//! swapping the injected dependency.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::venue::{
    AdapterError, FillReport, OrderIntent, PositionView, VenueAdapter, VenueSnapshot,
};
use bot_types::Venue;

/// Latency distribution to sample from.
#[derive(Debug, Clone)]
pub enum LatencyProfile {
    /// Constant delay on every call.
    Constant(Duration),
    /// Uniform distribution in `[min, max]`. Deterministic LCG-driven so
    /// tests are reproducible given the same seed.
    Uniform { min: Duration, max: Duration },
    /// Replay a fixed sequence; wraps around at the end.
    Sequence(Vec<Duration>),
    /// Disabled (no delay injected).
    None,
}

/// Outcome override for a single fetch.
#[derive(Debug, Clone)]
pub enum FailureProfile {
    /// Every Nth call fails with a `Network` error.
    EveryNth { every: u64, start: u64 },
    /// Scripted sequence of Ok/Err.
    Scripted(Vec<bool>), // true = Ok, false = Err
    /// Never inject failures.
    None,
}

pub struct ChaosAdapter {
    inner: Arc<dyn VenueAdapter>,
    latency: LatencyProfile,
    failure: FailureProfile,
    /// Monotonic call counter used for deterministic profile stepping.
    calls: AtomicU64,
    /// LCG state for Uniform profile.
    rng_state: std::sync::Mutex<u64>,
}

impl ChaosAdapter {
    pub fn new(
        inner: Arc<dyn VenueAdapter>,
        latency: LatencyProfile,
        failure: FailureProfile,
    ) -> Self {
        Self::with_seed(inner, latency, failure, 42)
    }

    pub fn with_seed(
        inner: Arc<dyn VenueAdapter>,
        latency: LatencyProfile,
        failure: FailureProfile,
        seed: u64,
    ) -> Self {
        Self {
            inner,
            latency,
            failure,
            calls: AtomicU64::new(0),
            rng_state: std::sync::Mutex::new(seed),
        }
    }

    /// Venue-profile defaults calibrated from 2026-Q1 probe data.
    pub fn pacifica_like(inner: Arc<dyn VenueAdapter>) -> Self {
        Self::new(
            inner,
            LatencyProfile::Uniform {
                min: Duration::from_millis(200),
                max: Duration::from_millis(400),
            },
            FailureProfile::EveryNth {
                every: 200,
                start: 100,
            },
        )
    }

    fn next_latency(&self, call_idx: u64) -> Duration {
        match &self.latency {
            LatencyProfile::None => Duration::ZERO,
            LatencyProfile::Constant(d) => *d,
            LatencyProfile::Uniform { min, max } => {
                let mut s = self.rng_state.lock().unwrap();
                *s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let u = (*s >> 11) as f64 / (1u64 << 53) as f64;
                let min_ms = min.as_millis() as u64;
                let max_ms = max.as_millis() as u64;
                let span = max_ms.saturating_sub(min_ms);
                let jitter = (u * span as f64) as u64;
                Duration::from_millis(min_ms + jitter)
            }
            LatencyProfile::Sequence(seq) => {
                if seq.is_empty() {
                    Duration::ZERO
                } else {
                    seq[(call_idx as usize) % seq.len()]
                }
            }
        }
    }

    fn should_fail(&self, call_idx: u64) -> bool {
        match &self.failure {
            FailureProfile::None => false,
            FailureProfile::EveryNth { every, start } => {
                *every > 0 && call_idx >= *start && (call_idx - *start) % *every == 0
            }
            FailureProfile::Scripted(seq) => {
                if seq.is_empty() {
                    false
                } else {
                    !seq[(call_idx as usize) % seq.len()]
                }
            }
        }
    }
}

#[async_trait]
impl VenueAdapter for ChaosAdapter {
    fn venue(&self) -> Venue {
        self.inner.venue()
    }

    async fn fetch_snapshot(&self, symbol: &str) -> Result<VenueSnapshot, AdapterError> {
        let idx = self.calls.fetch_add(1, Ordering::Relaxed);
        let delay = self.next_latency(idx);
        if delay > Duration::ZERO {
            tokio::time::sleep(delay).await;
        }
        if self.should_fail(idx) {
            return Err(AdapterError::Network(format!(
                "chaos: scripted failure on call #{idx}"
            )));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dryrun::DryRunVenueAdapter;
    use std::path::PathBuf;

    fn fixture_adapter() -> Arc<dyn VenueAdapter> {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("dryrun");
        Arc::new(DryRunVenueAdapter::new(Venue::Hyperliquid, fixture_dir))
    }

    #[tokio::test]
    async fn constant_latency_applies() {
        let chaos = ChaosAdapter::new(
            fixture_adapter(),
            LatencyProfile::Constant(Duration::from_millis(50)),
            FailureProfile::None,
        );
        let t0 = std::time::Instant::now();
        let _ = chaos.fetch_snapshot("BTC").await;
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= Duration::from_millis(45),
            "expected ≥ 45ms, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn uniform_latency_stays_in_bounds() {
        let chaos = ChaosAdapter::new(
            fixture_adapter(),
            LatencyProfile::Uniform {
                min: Duration::from_millis(10),
                max: Duration::from_millis(30),
            },
            FailureProfile::None,
        );
        for _ in 0..10 {
            let t0 = std::time::Instant::now();
            let _ = chaos.fetch_snapshot("BTC").await;
            let elapsed = t0.elapsed();
            assert!(
                elapsed >= Duration::from_millis(9),
                "too fast: {:?}",
                elapsed
            );
            // Allow tolerance on high side for async scheduling jitter.
            assert!(
                elapsed <= Duration::from_millis(100),
                "too slow: {:?}",
                elapsed
            );
        }
    }

    #[tokio::test]
    async fn every_nth_failure_profile_triggers_on_schedule() {
        let chaos = ChaosAdapter::new(
            fixture_adapter(),
            LatencyProfile::None,
            FailureProfile::EveryNth { every: 3, start: 0 },
        );
        let mut failures = 0;
        let mut successes = 0;
        for _ in 0..9 {
            match chaos.fetch_snapshot("BTC").await {
                Ok(_) => successes += 1,
                Err(_) => failures += 1,
            }
        }
        // calls 0,3,6 fail; 1,2,4,5,7,8 succeed
        assert_eq!(failures, 3);
        assert_eq!(successes, 6);
    }

    #[tokio::test]
    async fn scripted_failure_profile_reproducible() {
        let chaos = ChaosAdapter::new(
            fixture_adapter(),
            LatencyProfile::None,
            FailureProfile::Scripted(vec![true, true, false, true, false]),
        );
        let outcomes: Vec<bool> =
            futures_util::future::join_all((0..5).map(|_| chaos.fetch_snapshot("BTC")))
                .await
                .into_iter()
                .map(|r| r.is_ok())
                .collect();
        // join_all preserves order, so outcomes match the script.
        assert_eq!(outcomes, vec![true, true, false, true, false]);
    }

    #[tokio::test]
    async fn no_chaos_passes_through() {
        let chaos = ChaosAdapter::new(
            fixture_adapter(),
            LatencyProfile::None,
            FailureProfile::None,
        );
        let result = chaos.fetch_snapshot("BTC").await;
        assert!(result.is_ok());
    }
}
