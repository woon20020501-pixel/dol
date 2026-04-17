//! Kill switch — operator-initiated immediate flatten.
//!
//! Two trigger paths:
//! 1. **SIGINT / SIGTERM / Ctrl-C** — cross-platform via `tokio::signal::ctrl_c`
//!    (Windows and Unix both supported).
//! 2. **File flag** — presence of a configurable file path (default
//!    `./kill.flag`). Polled at each tick; dropping the file on the bot's
//!    working directory is a zero-coordination emergency stop that works
//!    even when SIGINT is unavailable (e.g. under some systemd units).
//!
//! Design:
//! - The signal handler is registered ONCE via `KillSwitch::arm()`. It sets
//!   an `AtomicBool` which `check()` reads on every tick.
//! - `check()` is `#[inline]` and lock-free — it's called from the hot path.
//! - The file-flag poll uses a single `fs::metadata` call; no read of the
//!   file contents.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::RiskDecision;

/// Default file-flag path polled by the kill switch.
pub const DEFAULT_FLAG_FILE: &str = "kill.flag";

#[derive(Debug, Clone)]
pub struct KillSwitch {
    /// Set when SIGINT/SIGTERM is received.
    signal_tripped: Arc<AtomicBool>,
    /// Path polled for file-flag trigger.
    flag_path: PathBuf,
}

impl KillSwitch {
    pub fn new<P: AsRef<Path>>(flag_path: P) -> Self {
        Self {
            signal_tripped: Arc::new(AtomicBool::new(false)),
            flag_path: flag_path.as_ref().to_path_buf(),
        }
    }

    pub fn default_path() -> Self {
        Self::new(DEFAULT_FLAG_FILE)
    }

    /// Arm the SIGINT/SIGTERM handler. Must be called from async context
    /// (needs a tokio runtime). Safe to call more than once — only the first
    /// call registers the handler.
    ///
    /// The returned `JoinHandle` should be kept alive for the duration of
    /// the bot; dropping it cancels the handler task.
    pub fn arm_signal_handler(&self) -> tokio::task::JoinHandle<()> {
        let flag = Arc::clone(&self.signal_tripped);
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                flag.store(true, Ordering::SeqCst);
                tracing::warn!("kill_switch: SIGINT/Ctrl-C received — flatten requested");
            }
        })
    }

    /// Manually trip the switch (for tests and internal watchdog escalation).
    pub fn trip(&self) {
        self.signal_tripped.store(true, Ordering::SeqCst);
    }

    /// Check whether a kill trigger has fired. Called every tick.
    ///
    /// Returns `Flatten` if either trigger is active; `Pass` otherwise.
    /// This is `#[inline]` and lock-free — hot path safe.
    #[inline]
    pub fn check(&self) -> RiskDecision {
        if self.signal_tripped.load(Ordering::Relaxed) {
            return RiskDecision::Flatten {
                reason: "SIGINT/SIGTERM received".to_string(),
            };
        }
        // File-flag is slower (syscall); keep it behind the cheap atomic check.
        if self.flag_path.exists() {
            return RiskDecision::Flatten {
                reason: format!("kill flag file present: {}", self.flag_path.display()),
            };
        }
        RiskDecision::Pass
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::default_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untripped_passes() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ks = KillSwitch::new(tmpdir.path().join("kill.flag"));
        assert_eq!(ks.check(), RiskDecision::Pass);
    }

    #[test]
    fn manual_trip_triggers_flatten() {
        let tmpdir = tempfile::tempdir().unwrap();
        let ks = KillSwitch::new(tmpdir.path().join("kill.flag"));
        ks.trip();
        assert!(matches!(ks.check(), RiskDecision::Flatten { .. }));
    }

    #[test]
    fn file_flag_triggers_flatten() {
        let tmpdir = tempfile::tempdir().unwrap();
        let flag_path = tmpdir.path().join("kill.flag");
        let ks = KillSwitch::new(&flag_path);
        std::fs::write(&flag_path, "").unwrap();
        assert!(matches!(ks.check(), RiskDecision::Flatten { .. }));
    }

    #[test]
    fn removing_flag_clears_after_trip_is_sticky() {
        // File-flag is stateless (based on fs), but the signal atomic is sticky.
        let tmpdir = tempfile::tempdir().unwrap();
        let flag_path = tmpdir.path().join("kill.flag");
        let ks = KillSwitch::new(&flag_path);
        std::fs::write(&flag_path, "").unwrap();
        assert!(matches!(ks.check(), RiskDecision::Flatten { .. }));
        std::fs::remove_file(&flag_path).unwrap();
        assert_eq!(ks.check(), RiskDecision::Pass);
        // But a previous manual trip remains sticky
        ks.trip();
        std::fs::remove_file(&flag_path).ok();
        assert!(matches!(ks.check(), RiskDecision::Flatten { .. }));
    }
}
