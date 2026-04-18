//! `bot-tests` — Parity fixture harness for the Dol v4 framework.
//!
//! Loads JSON fixture files from `DOL_FIXTURES_DIR` (env var) or the
//! repo-relative path `../../../strategy/rust_fixtures/` (resolved
//! relative to `CARGO_MANIFEST_DIR`).

use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Generic fixture envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Fixture<I, E> {
    pub name: String,
    pub input: I,
    pub expected: E,
    pub tolerance: f64,
    #[serde(default)]
    pub notes: String,
}

// ---------------------------------------------------------------------------
// Fixture directory resolution
// ---------------------------------------------------------------------------

/// Resolve the path to the `rust_fixtures/` directory.
///
/// Priority:
/// 1. `DOL_FIXTURES_DIR` environment variable (if set and non-empty).
/// 2. Walk up from `CARGO_MANIFEST_DIR` three levels, then
///    `strategy/rust_fixtures/`.
///
/// Panics with an actionable message if neither path resolves to a directory.
pub fn fixtures_dir() -> PathBuf {
    // 1. Env var override
    if let Ok(dir) = std::env::var("DOL_FIXTURES_DIR") {
        if !dir.is_empty() {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                return p;
            }
            panic!(
                "DOL_FIXTURES_DIR is set to {:?} but it is not a directory.",
                p
            );
        }
    }

    // 2. Repo-relative fallback: CARGO_MANIFEST_DIR / ../../../strategy/rust_fixtures
    //    crates/bot-tests → crates → bot-rs → repo root (monorepo layout),
    //    then strategy/rust_fixtures
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| panic!("CARGO_MANIFEST_DIR is not set; run tests via `cargo test`."));
    let p = PathBuf::from(manifest)
        .join("..") // crates
        .join("..") // bot-rs
        .join("..") // repo root
        .join("strategy")
        .join("rust_fixtures");

    // Canonicalize to resolve `..` components
    let canonical = p.canonicalize().unwrap_or_else(|e| {
        panic!(
            "Could not resolve fixture directory at {:?}: {}.\n\
             Set the DOL_FIXTURES_DIR environment variable to the absolute \
             path of the rust_fixtures/ directory.",
            p, e
        )
    });
    if !canonical.is_dir() {
        panic!(
            "Resolved fixture path {:?} is not a directory.\n\
             Set DOL_FIXTURES_DIR to the correct path.",
            canonical
        );
    }
    canonical
}

/// Load all fixture cases from `<fixtures_dir>/<section>.json`.
pub fn load_fixtures<I, E>(section: &str) -> Vec<Fixture<I, E>>
where
    I: DeserializeOwned,
    E: DeserializeOwned,
{
    let path = fixtures_dir().join(format!("{}.json", section));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read fixture file {:?}: {}", path, e));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("Cannot parse fixture file {:?}: {}", path, e))
}

// ---------------------------------------------------------------------------
// Float helpers
// ---------------------------------------------------------------------------

/// Decode a JSON value as f64, treating `"inf"`, `"-inf"`, `"nan"` strings.
pub fn parse_float_or_special(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().expect("JSON number out of f64 range"),
        serde_json::Value::String(s) => match s.as_str() {
            "inf" => f64::INFINITY,
            "-inf" => f64::NEG_INFINITY,
            "nan" => f64::NAN,
            other => panic!("Unrecognised float string {:?}", other),
        },
        serde_json::Value::Null => f64::NAN, // null encodes None / error result
        other => panic!("Expected number or special string, got {:?}", other),
    }
}

/// Assert that `actual` is within `tol` of `expected`, with correct NaN and ±inf handling.
///
/// - NaN == NaN is considered equal (parity check only).
/// - ±inf must match sign exactly.
/// - Finite values use absolute tolerance.
pub fn assert_close(actual: f64, expected: f64, tol: f64, case_name: &str) {
    if expected.is_nan() {
        assert!(
            actual.is_nan(),
            "case '{}': expected NaN, got {}",
            case_name,
            actual
        );
        return;
    }
    if expected.is_infinite() {
        assert!(
            actual.is_infinite() && actual.signum() == expected.signum(),
            "case '{}': expected {}, got {}",
            case_name,
            expected,
            actual
        );
        return;
    }
    let diff = (actual - expected).abs();
    assert!(
        diff <= tol,
        "case '{}': expected {}, got {}, diff {}, tol {}",
        case_name,
        expected,
        actual,
        diff,
        tol
    );
}
