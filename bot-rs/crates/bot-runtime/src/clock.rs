//! Simulated-time clock for the accelerated NAV demo.
//!
//! The affine transform:
//! ```text
//! simulated_now = real_start + (real_now - real_start) * factor
//! ```
//!
//! With `factor = 1` (default) the simulated clock tracks wall clock.
//! With `factor = 3600` one real second advances the simulated clock
//! by one simulated hour — a 60-second demo shows ~60 h of NAV accrual.
//!
//! **Do NOT use this clock for live network I/O** (Pacifica API fetches,
//! etc.). Per integration-spec: adapters use their own wall clock so that
//! real-time market data is never filtered through a simulated timestamp.

/// Affine simulated clock.
///
/// Simulated time is pinned to the real instant at construction and
/// advanced by `factor` for each real elapsed second.
pub struct SimulatedClock {
    real_start: std::time::Instant,
    /// Unix milliseconds at which the clock was constructed (real wall clock).
    pub real_start_ms: i64,
    factor: f64,
}

impl SimulatedClock {
    /// Construct a new `SimulatedClock` with the given acceleration factor.
    ///
    /// # Panics
    ///
    /// Panics if `factor <= 0.0`.
    pub fn new(factor: f64) -> Self {
        assert!(factor > 0.0, "accel factor must be > 0, got {}", factor);
        Self {
            real_start: std::time::Instant::now(),
            real_start_ms: chrono::Utc::now().timestamp_millis(),
            factor,
        }
    }

    /// Simulated now as Unix milliseconds.
    ///
    /// `simulated_ms = real_start_ms + real_elapsed_ms * factor`
    pub fn now_ms(&self) -> i64 {
        let real_elapsed_ms = self.real_start.elapsed().as_millis() as f64;
        self.real_start_ms + (real_elapsed_ms * self.factor) as i64
    }

    /// Simulated now as Unix seconds (f64 for sub-second precision).
    pub fn now_s(&self) -> f64 {
        self.now_ms() as f64 / 1000.0
    }

    /// Convert a real elapsed duration to simulated seconds.
    ///
    /// `simulated_dt = real_dt_seconds * factor`
    pub fn simulated_dt_seconds(&self, real_dt: std::time::Duration) -> f64 {
        real_dt.as_secs_f64() * self.factor
    }

    /// The acceleration factor.
    pub fn factor(&self) -> f64 {
        self.factor
    }

    /// The current simulated instant wrapped as a `chrono::DateTime<Utc>`.
    ///
    /// Useful when emitting signal JSON `ts_unix` / filenames — pass this
    /// to `emit_signal` instead of `Utc::now()`.
    pub fn now_datetime(&self) -> chrono::DateTime<chrono::Utc> {
        let now_ms = self.now_ms();
        let secs = now_ms / 1000;
        let nanos = ((now_ms % 1000) * 1_000_000) as u32;
        chrono::DateTime::from_timestamp(secs, nanos).unwrap_or_else(chrono::Utc::now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn factor_one_tracks_real_time() {
        let clock = SimulatedClock::new(1.0);
        // Simulated now should be very close to real_start_ms (within 50 ms
        // of construction, since no real time has elapsed).
        let sim = clock.now_ms();
        let delta = (sim - clock.real_start_ms).abs();
        assert!(
            delta < 50,
            "factor=1 simulated ms should track real ms within 50ms, got delta={}",
            delta
        );
    }

    #[test]
    fn factor_3600_advances_one_simulated_second_per_real_millisecond() {
        // We can't actually sleep 1s in a unit test, so we test via
        // `simulated_dt_seconds` which is pure arithmetic.
        let clock = SimulatedClock::new(3600.0);
        let sim_dt = clock.simulated_dt_seconds(Duration::from_secs(1));
        assert!(
            (sim_dt - 3600.0).abs() < 1e-9,
            "factor=3600 simulated_dt(1s) should == 3600.0, got {}",
            sim_dt
        );
    }

    #[test]
    fn simulated_dt_seconds_scales_by_factor() {
        let clock = SimulatedClock::new(3600.0);
        // 2 real seconds → 7200 simulated seconds
        let sim_dt = clock.simulated_dt_seconds(Duration::from_secs(2));
        assert!((sim_dt - 7200.0).abs() < 1e-9);

        // 500 ms real → 1800 simulated seconds
        let sim_dt_half = clock.simulated_dt_seconds(Duration::from_millis(500));
        assert!((sim_dt_half - 1800.0).abs() < 1e-6);
    }

    #[test]
    fn factor_one_simulated_dt_equals_real_dt() {
        let clock = SimulatedClock::new(1.0);
        let dt = clock.simulated_dt_seconds(Duration::from_secs(10));
        assert!((dt - 10.0).abs() < 1e-9);
    }

    #[test]
    fn now_datetime_is_close_to_real_start() {
        let clock = SimulatedClock::new(1.0);
        let dt = clock.now_datetime();
        let real_now = chrono::Utc::now();
        // Difference should be small (< 100ms)
        let diff_ms = (real_now.timestamp_millis() - dt.timestamp_millis()).abs();
        assert!(
            diff_ms < 100,
            "now_datetime with factor=1 should be close to real time, diff={}ms",
            diff_ms
        );
    }

    #[test]
    #[should_panic(expected = "accel factor must be > 0")]
    fn panics_on_zero_factor() {
        let _ = SimulatedClock::new(0.0);
    }

    #[test]
    #[should_panic(expected = "accel factor must be > 0")]
    fn panics_on_negative_factor() {
        let _ = SimulatedClock::new(-1.0);
    }

    #[test]
    fn factor_accessor_returns_correct_value() {
        let clock = SimulatedClock::new(42.5);
        assert!((clock.factor() - 42.5).abs() < 1e-12);
    }
}
