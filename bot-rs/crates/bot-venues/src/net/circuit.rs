use std::time::{Duration, Instant};

/// Circuit breaker states — mirrors TS `net/circuit.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation. WS preferred.
    Closed,
    /// Too many failures. REST fallback only.
    Open,
    /// Cooling off. Next request is a probe — success → Closed,
    /// failure → back to Open.
    HalfOpen,
}

/// Circuit breaker for venue adapters.
///
/// When the WS connection fails repeatedly, the circuit opens and
/// the adapter falls back to REST polling. After `reset_timeout`
/// the circuit transitions to HalfOpen, allowing one probe request.
pub struct CircuitBreaker {
    state: CircuitState,
    failures: u32,
    threshold: u32,
    reset_timeout: Duration,
    opened_at: Option<Instant>,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            state: CircuitState::Closed,
            failures: 0,
            threshold,
            reset_timeout,
            opened_at: None,
        }
    }

    /// Default production: 5 failures, 120s reset.
    pub fn default_production() -> Self {
        Self::new(5, Duration::from_secs(120))
    }

    pub fn state(&self) -> CircuitState {
        self.state
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Whether a request should be allowed through.
    /// Automatically transitions Open → HalfOpen after timeout.
    pub fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true,
            CircuitState::Open => {
                if let Some(opened) = self.opened_at {
                    if opened.elapsed() >= self.reset_timeout {
                        self.state = CircuitState::HalfOpen;
                        tracing::info!("circuit breaker: OPEN → HALF_OPEN");
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Record a successful request. Resets the breaker.
    pub fn record_success(&mut self) {
        if self.state != CircuitState::Closed {
            tracing::info!(
                prev = ?self.state,
                "circuit breaker: → CLOSED (success)"
            );
        }
        self.failures = 0;
        self.state = CircuitState::Closed;
        self.opened_at = None;
    }

    /// Record a failed request. Opens the breaker after threshold.
    pub fn record_failure(&mut self) {
        self.failures += 1;

        if self.state == CircuitState::HalfOpen || self.failures >= self.threshold {
            if self.state != CircuitState::Open {
                tracing::warn!(
                    failures = self.failures,
                    threshold = self.threshold,
                    "circuit breaker: → OPEN"
                );
            }
            self.state = CircuitState::Open;
            self.opened_at = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed() {
        let mut cb = CircuitBreaker::new(5, Duration::from_secs(120));
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn opens_after_threshold_failures() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(120));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());

        cb.record_failure(); // 3rd = threshold
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn success_resets_to_closed() {
        let mut cb = CircuitBreaker::new(2, Duration::from_secs(120));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Simulate time passing (we can't easily in unit test, so
        // manually set state for the success path test)
        cb.state = CircuitState::HalfOpen;
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failures(), 0);
    }

    #[test]
    fn half_open_failure_reopens() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(120));
        cb.state = CircuitState::HalfOpen;
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn failures_below_threshold_stay_closed() {
        let mut cb = CircuitBreaker::new(5, Duration::from_secs(120));
        for _ in 0..4 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn success_mid_failures_resets_count() {
        let mut cb = CircuitBreaker::new(5, Duration::from_secs(120));
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.failures(), 0);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn open_transitions_to_half_open_after_timeout() {
        let mut cb = CircuitBreaker::new(2, Duration::from_secs(0)); // instant timeout
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // With 0s timeout, allow_request should immediately transition
        assert!(cb.allow_request());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }
}
