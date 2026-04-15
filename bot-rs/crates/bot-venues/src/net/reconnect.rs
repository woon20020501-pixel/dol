use std::time::Duration;

use rand::Rng;

/// Exponential backoff with full jitter.
///
/// Mirrors TS `net/reconnect.ts`. Sequence (no jitter):
/// 1s → 2s → 4s → 8s → 16s → 32s → 60s (capped).
/// With jitter: uniform random in `[0, base)`.
pub struct Reconnect {
    initial_ms: u64,
    max_ms: u64,
    jitter: bool,
    attempt: u32,
}

impl Reconnect {
    pub fn new(initial_ms: u64, max_ms: u64, jitter: bool) -> Self {
        Self {
            initial_ms,
            max_ms,
            jitter,
            attempt: 0,
        }
    }

    /// Default production settings: 1s initial, 60s max, jitter on.
    pub fn default_production() -> Self {
        Self::new(1_000, 60_000, true)
    }

    /// Compute next delay and advance the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let exp = self.attempt.min(20);
        let base = self.initial_ms.saturating_mul(1u64 << exp);
        let capped = base.min(self.max_ms);
        self.attempt += 1;

        if self.jitter && capped > 0 {
            let jittered = rand::thread_rng().gen_range(0..capped);
            Duration::from_millis(jittered)
        } else {
            Duration::from_millis(capped)
        }
    }

    /// Reset after a successful connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn attempts(&self) -> u32 {
        self.attempt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence_no_jitter() {
        let mut r = Reconnect::new(1_000, 60_000, false);
        assert_eq!(r.next_delay(), Duration::from_millis(1_000));
        assert_eq!(r.next_delay(), Duration::from_millis(2_000));
        assert_eq!(r.next_delay(), Duration::from_millis(4_000));
        assert_eq!(r.next_delay(), Duration::from_millis(8_000));
        assert_eq!(r.next_delay(), Duration::from_millis(16_000));
        assert_eq!(r.next_delay(), Duration::from_millis(32_000));
        assert_eq!(r.next_delay(), Duration::from_millis(60_000)); // capped
        assert_eq!(r.next_delay(), Duration::from_millis(60_000)); // stays capped
    }

    #[test]
    fn backoff_with_jitter_stays_in_range() {
        let mut r = Reconnect::new(1_000, 60_000, true);
        for _ in 0..100 {
            let delay = r.next_delay();
            assert!(delay < Duration::from_millis(60_001));
        }
    }

    #[test]
    fn reset_restarts_sequence() {
        let mut r = Reconnect::new(1_000, 60_000, false);
        r.next_delay(); // 1s
        r.next_delay(); // 2s
        assert_eq!(r.attempts(), 2);
        r.reset();
        assert_eq!(r.attempts(), 0);
        assert_eq!(r.next_delay(), Duration::from_millis(1_000));
    }

    #[test]
    fn zero_initial_yields_zero() {
        let mut r = Reconnect::new(0, 60_000, false);
        assert_eq!(r.next_delay(), Duration::from_millis(0));
    }

    #[test]
    fn attempt_counter_increments() {
        let mut r = Reconnect::new(1_000, 60_000, false);
        assert_eq!(r.attempts(), 0);
        r.next_delay();
        assert_eq!(r.attempts(), 1);
        r.next_delay();
        assert_eq!(r.attempts(), 2);
    }
}
