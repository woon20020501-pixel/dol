//! Unit tests for `PacificaAuthenticatedAdapter`.
//!
//! All tests here run by default (no `#[ignore]`). They exercise:
//! - Credential validation (empty key / empty builder code)
//! - Redacted `Debug` output (API key must never appear)
//! - `builder_code()` accessor
//! - `from_env()` behaviour with and without env vars
//!
//! No network calls are made in this file.

use bot_adapters::pacifica_auth::{PacificaAuthenticatedAdapter, ENV_API_KEY, ENV_BUILDER_CODE};
use std::sync::Mutex;

// Mutex to serialize env-var tests so they do not race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ─────────────────────────────────────────────────────────────────────────────
// Constructor + credential validation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn new_constructs_with_valid_credentials() {
    let adapter = PacificaAuthenticatedAdapter::new("fake-key".into(), "BLDR42".into()).unwrap();
    assert_eq!(adapter.builder_code(), "BLDR42");
}

#[test]
fn new_rejects_empty_api_key() {
    let err = PacificaAuthenticatedAdapter::new("".into(), "BLDR42".into()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("PACIFICA_API_KEY") || msg.to_lowercase().contains("empty"),
        "Error message should mention the env var name or 'empty': {msg}"
    );
}

#[test]
fn new_rejects_empty_builder_code() {
    let err = PacificaAuthenticatedAdapter::new("some-key".into(), "".into()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("PACIFICA_BUILDER_CODE") || msg.to_lowercase().contains("empty"),
        "Error message should mention the env var name or 'empty': {msg}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Debug redaction — API key MUST NOT appear in debug output
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn debug_format_contains_redacted_and_not_the_key() {
    let adapter =
        PacificaAuthenticatedAdapter::new("super-secret-key-xyz".into(), "BLDR42".into()).unwrap();
    let dbg = format!("{:?}", adapter);

    // Must contain the redaction marker.
    assert!(
        dbg.contains("<redacted>"),
        "Debug output must contain '<redacted>', got: {dbg}"
    );

    // Must NOT leak the actual API key.
    assert!(
        !dbg.contains("super-secret-key-xyz"),
        "Debug output MUST NOT contain the API key, got: {dbg}"
    );

    // Builder code is public and may appear.
    assert!(
        dbg.contains("BLDR42"),
        "Debug output should show builder_code, got: {dbg}"
    );
}

#[test]
fn debug_format_never_leaks_any_credential_substring() {
    let adapter =
        PacificaAuthenticatedAdapter::new("tok_abc123def456".into(), "PUBLIC_BLD".into()).unwrap();
    let dbg = format!("{:?}", adapter);

    // None of these substrings of the key should appear.
    for substr in &["tok_abc123def456", "abc123def456", "tok_abc"] {
        assert!(
            !dbg.contains(substr),
            "Debug output leaked credential substring '{substr}': {dbg}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// from_env()
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn from_env_fails_when_api_key_is_unset() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Ensure the env var is absent (don't mutate if set by outer test harness).
    let _guard_key = EnvGuard::unset(ENV_API_KEY);
    let _guard_bc = EnvGuard::unset(ENV_BUILDER_CODE);

    let err = PacificaAuthenticatedAdapter::from_env().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains(ENV_API_KEY),
        "Error message should name the missing env var, got: {msg}"
    );
}

#[test]
fn from_env_fails_when_builder_code_is_unset() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard_key = EnvGuard::set(ENV_API_KEY, "some-key");
    let _guard_bc = EnvGuard::unset(ENV_BUILDER_CODE);

    let err = PacificaAuthenticatedAdapter::from_env().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains(ENV_BUILDER_CODE),
        "Error message should name the missing env var, got: {msg}"
    );
}

#[test]
fn from_env_succeeds_when_both_vars_are_set() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard_key = EnvGuard::set(ENV_API_KEY, "test-api-key");
    let _guard_bc = EnvGuard::set(ENV_BUILDER_CODE, "TEST_BLDR");

    let adapter = PacificaAuthenticatedAdapter::from_env()
        .expect("from_env should succeed when both env vars are set");
    assert_eq!(adapter.builder_code(), "TEST_BLDR");
}

#[test]
fn from_env_fails_when_api_key_is_empty_string() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard_key = EnvGuard::set(ENV_API_KEY, "");
    let _guard_bc = EnvGuard::set(ENV_BUILDER_CODE, "TEST_BLDR");

    let err = PacificaAuthenticatedAdapter::from_env().unwrap_err();
    // Could be missing-var error (empty var may act as unset depending on OS) or empty-key error.
    let _msg = err.to_string(); // Just confirm it doesn't panic.
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: RAII env var guard for isolated tests
// ─────────────────────────────────────────────────────────────────────────────

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        // Safety: test-only, single-threaded Rust test runner is sufficient.
        unsafe { std::env::set_var(key, value) };
        EnvGuard { key, original }
    }

    fn unset(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        EnvGuard { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}
