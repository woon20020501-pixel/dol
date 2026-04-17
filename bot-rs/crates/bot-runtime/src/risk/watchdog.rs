//! API-latency watchdog.
//!
//! Monitors the rolling-window P99 latency of venue REST calls and escalates
//! when sustained degradation crosses operator thresholds.
//!
//! Reference: Nygard (2007), "Release It!" §5 — bulkheads and timeouts; and
//! Dean & Barroso (2013), "The Tail at Scale", CACM 56(2):74-80 — P99 as the
//! canonical latency SLO signal at the request tail.
//!
//! Policy (defaults):
//! - `latency_p99 < warn_threshold (1.5s)`       → Pass
//! - `warn ≤ latency_p99 < fatal (3.0s)`         → Reduce 0.5
//! - `latency_p99 ≥ fatal_threshold (3.0s)` held for `sustained_window (30s)` → Flatten
//!
//! Implementation: ring buffer of recent (ts, latency_ms) samples, ≤ 1024
//! entries. Eviction keyed on time (drop older than `window_ms`).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::RiskDecision;

pub const DEFAULT_WARN_THRESHOLD: Duration = Duration::from_millis(1_500);
pub const DEFAULT_FATAL_THRESHOLD: Duration = Duration::from_millis(3_000);
pub const DEFAULT_SUSTAINED_WINDOW: Duration = Duration::from_secs(30);
pub const DEFAULT_OBSERVATION_WINDOW: Duration = Duration::from_secs(60);
pub const DEFAULT_MAX_SAMPLES: usize = 1024;

#[derive(Debug, Clone)]
struct LatencySample {
    ts: Instant,
    latency: Duration,
}

#[derive(Debug, Clone)]
pub struct ApiLatencyWatchdog {
    samples: VecDeque<LatencySample>,
    warn_threshold: Duration,
    fatal_threshold: Duration,
    sustained_window: Duration,
    observation_window: Duration,
    /// Timestamp at which the fatal threshold was first crossed and not
    /// subsequently cleared.
    fatal_since: Option<Instant>,
    capacity: usize,
}

impl ApiLatencyWatchdog {
    pub fn new() -> Self {
        Self::with_params(
            DEFAULT_WARN_THRESHOLD,
            DEFAULT_FATAL_THRESHOLD,
            DEFAULT_SUSTAINED_WINDOW,
            DEFAULT_OBSERVATION_WINDOW,
            DEFAULT_MAX_SAMPLES,
        )
    }

    pub fn with_params(
        warn: Duration,
        fatal: Duration,
        sustained: Duration,
        observe: Duration,
        capacity: usize,
    ) -> Self {
        assert!(warn > Duration::ZERO && fatal > warn);
        assert!(sustained > Duration::ZERO);
        assert!(observe >= sustained);
        assert!(capacity >= 4);
        Self {
            samples: VecDeque::with_capacity(capacity),
            warn_threshold: warn,
            fatal_threshold: fatal,
            sustained_window: sustained,
            observation_window: observe,
            fatal_since: None,
            capacity,
        }
    }

    /// Record a new latency observation.
    pub fn record(&mut self, ts: Instant, latency: Duration) {
        // Evict samples outside the observation window.
        while let Some(front) = self.samples.front() {
            if ts.saturating_duration_since(front.ts) > self.observation_window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(LatencySample { ts, latency });

        // Update fatal_since sticky flag.
        let p99 = self.p99_latency();
        if p99 >= self.fatal_threshold {
            if self.fatal_since.is_none() {
                self.fatal_since = Some(ts);
            }
        } else {
            self.fatal_since = None;
        }
    }

    /// P99 latency over the current observation window. Returns zero when
    /// empty.
    pub fn p99_latency(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let mut vals: Vec<Duration> = self.samples.iter().map(|s| s.latency).collect();
        vals.sort();
        // 99th percentile index (ceil): with n=100, idx=99.
        let idx = ((vals.len() as f64 * 0.99).ceil() as usize)
            .saturating_sub(1)
            .min(vals.len() - 1);
        vals[idx]
    }

    /// Decision for a candidate trade at `now`.
    pub fn check(&self, now: Instant) -> RiskDecision {
        let p99 = self.p99_latency();
        if p99 < self.warn_threshold {
            return RiskDecision::Pass;
        }
        if p99 < self.fatal_threshold {
            return RiskDecision::Reduce {
                size_multiplier: 0.5,
                reason: format!(
                    "API p99 latency {:.0}ms ≥ warn {:.0}ms",
                    p99.as_millis(),
                    self.warn_threshold.as_millis()
                ),
            };
        }
        // Fatal: require sustained (p99 above fatal AND fatal_since set more
        // than `sustained_window` ago) before flattening.
        if let Some(fs) = self.fatal_since {
            let sustained_for = now.saturating_duration_since(fs);
            if sustained_for >= self.sustained_window {
                return RiskDecision::Flatten {
                    reason: format!(
                        "API p99 latency {:.0}ms sustained ≥ {:.0}s",
                        p99.as_millis(),
                        self.sustained_window.as_secs_f64()
                    ),
                };
            }
        }
        // Below sustained threshold but above fatal → still reduce hard.
        RiskDecision::Reduce {
            size_multiplier: 0.25,
            reason: format!(
                "API p99 latency {:.0}ms ≥ fatal {:.0}ms (pre-sustained)",
                p99.as_millis(),
                self.fatal_threshold.as_millis()
            ),
        }
    }
}

impl Default for ApiLatencyWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_pass() {
        let w = ApiLatencyWatchdog::new();
        assert_eq!(w.check(Instant::now()), RiskDecision::Pass);
    }

    #[test]
    fn fast_api_passes() {
        let mut w = ApiLatencyWatchdog::new();
        let t = Instant::now();
        for i in 0..100 {
            w.record(t + Duration::from_millis(i * 10), Duration::from_millis(50));
        }
        assert_eq!(w.check(t + Duration::from_secs(1)), RiskDecision::Pass);
    }

    #[test]
    fn warn_triggers_reduce() {
        let mut w = ApiLatencyWatchdog::new();
        let t = Instant::now();
        for i in 0..100 {
            w.record(
                t + Duration::from_millis(i * 10),
                Duration::from_millis(1_800),
            );
        }
        match w.check(t + Duration::from_secs(1)) {
            RiskDecision::Reduce {
                size_multiplier, ..
            } => assert!(size_multiplier < 1.0),
            other => panic!("expected Reduce, got {:?}", other),
        }
    }

    #[test]
    fn fatal_below_sustained_is_reduce() {
        let mut w = ApiLatencyWatchdog::with_params(
            Duration::from_millis(1_500),
            Duration::from_millis(3_000),
            Duration::from_secs(30),
            Duration::from_secs(60),
            1024,
        );
        let t = Instant::now();
        for i in 0..50 {
            w.record(
                t + Duration::from_millis(i * 10),
                Duration::from_millis(3_500),
            );
        }
        // < 30s sustained → Reduce
        match w.check(t + Duration::from_secs(5)) {
            RiskDecision::Reduce { .. } => {}
            other => panic!("expected Reduce, got {:?}", other),
        }
    }

    #[test]
    fn fatal_above_sustained_is_flatten() {
        let mut w = ApiLatencyWatchdog::with_params(
            Duration::from_millis(1_500),
            Duration::from_millis(3_000),
            Duration::from_secs(5),
            Duration::from_secs(60),
            1024,
        );
        let t = Instant::now();
        // 60s of 4s latency
        for i in 0..60 {
            w.record(t + Duration::from_secs(i), Duration::from_millis(4_000));
        }
        let now = t + Duration::from_secs(61);
        assert!(matches!(w.check(now), RiskDecision::Flatten { .. }));
    }

    #[test]
    fn recovery_clears_fatal_since() {
        let mut w = ApiLatencyWatchdog::with_params(
            Duration::from_millis(1_500),
            Duration::from_millis(3_000),
            Duration::from_secs(5),
            Duration::from_secs(60), // observation window
            1024,
        );
        let t = Instant::now();
        // 20 slow samples over seconds [0..19].
        for i in 0..20 {
            w.record(t + Duration::from_secs(i), Duration::from_millis(4_000));
        }
        assert!(w.fatal_since.is_some());
        // Recovery: 120 fast samples over seconds [20..139]. At second 139,
        // all samples t<79 are evicted (gap > 60s), leaving only fast samples.
        for i in 20..140 {
            w.record(t + Duration::from_secs(i), Duration::from_millis(100));
        }
        assert!(
            w.fatal_since.is_none(),
            "fatal_since must clear on recovery"
        );
    }
}
