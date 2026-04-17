//! HTTP 429 rate-limit handling.
//!
//! # Spec
//!
//! - RFC 7231 §7.1.3 "Retry-After" — value is either:
//!   1. `delta-seconds` (non-negative decimal integer), OR
//!   2. HTTP-date (RFC 7231 §7.1.1.1, e.g. `Wed, 21 Oct 2015 07:28:00 GMT`).
//!
//! - RFC 6585 §4 defines the 429 status code with an optional
//!   `Retry-After` response header.
//!
//! # Behavior in this crate
//!
//! When a venue returns HTTP 429:
//! 1. Parse `Retry-After` (integer seconds OR RFC 7231 date) via
//!    [`parse_retry_after`]; fallback to `DEFAULT_BACKOFF_SECS` on missing/
//!    unparseable values.
//! 2. Cap at [`MAX_BACKOFF_SECS`] to prevent a pathological server from
//!    pinning the client indefinitely.
//! 3. Emit [`crate::venue::AdapterError::RateLimited`] with the computed wait.
//! 4. `OrderClient::submit_idempotent` switches from exponential backoff to
//!    the server-advised wait for that attempt.

use chrono::{DateTime, Utc};

/// Default wait when the server returns 429 without a `Retry-After` header.
/// Mirrors Cloudflare's common "30s" convention.
pub const DEFAULT_BACKOFF_SECS: u64 = 30;

/// Hard cap on `Retry-After` interpretation (60 seconds).
/// A misbehaving server returning "Retry-After: 99999" will be clamped here,
/// then the circuit breaker (watchdog) takes over.
pub const MAX_BACKOFF_SECS: u64 = 60;

/// Parse a `Retry-After` header value per RFC 7231 §7.1.3.
///
/// Handles:
/// - `"30"` → 30 seconds
/// - `"  30  "` → 30 seconds (whitespace trimmed)
/// - `"Wed, 21 Oct 2015 07:28:00 GMT"` → seconds until that moment (vs now)
/// - Missing / malformed → `DEFAULT_BACKOFF_SECS`
/// - Cap at `MAX_BACKOFF_SECS`
///
/// `now` is injected (instead of calling `Utc::now()` internally) so tests
/// can exercise both delta-seconds and HTTP-date paths deterministically.
pub fn parse_retry_after(header_value: Option<&str>, now: DateTime<Utc>) -> u64 {
    let Some(raw) = header_value else {
        return DEFAULT_BACKOFF_SECS;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_BACKOFF_SECS;
    }
    // Try integer seconds first (the fast, common path).
    if let Ok(secs) = trimmed.parse::<u64>() {
        return secs.min(MAX_BACKOFF_SECS);
    }
    // Try RFC 7231 HTTP-date (RFC 2822 / RFC 7231 preferred format).
    if let Ok(when) = DateTime::parse_from_rfc2822(trimmed) {
        let delta = when.signed_duration_since(now).num_seconds();
        if delta <= 0 {
            return 0;
        }
        return (delta as u64).min(MAX_BACKOFF_SECS);
    }
    DEFAULT_BACKOFF_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap()
    }

    #[test]
    fn integer_seconds_parsed() {
        assert_eq!(parse_retry_after(Some("30"), fixed_now()), 30);
        assert_eq!(parse_retry_after(Some("  5  "), fixed_now()), 5);
        assert_eq!(parse_retry_after(Some("0"), fixed_now()), 0);
    }

    #[test]
    fn missing_header_uses_default() {
        assert_eq!(parse_retry_after(None, fixed_now()), DEFAULT_BACKOFF_SECS);
    }

    #[test]
    fn empty_header_uses_default() {
        assert_eq!(
            parse_retry_after(Some(""), fixed_now()),
            DEFAULT_BACKOFF_SECS
        );
        assert_eq!(
            parse_retry_after(Some("   "), fixed_now()),
            DEFAULT_BACKOFF_SECS
        );
    }

    #[test]
    fn unparseable_header_uses_default() {
        assert_eq!(
            parse_retry_after(Some("not a number"), fixed_now()),
            DEFAULT_BACKOFF_SECS
        );
    }

    #[test]
    fn integer_capped_at_max() {
        assert_eq!(
            parse_retry_after(Some("999"), fixed_now()),
            MAX_BACKOFF_SECS
        );
    }

    #[test]
    fn http_date_in_future_computes_delta() {
        let now = fixed_now();
        let when = now + Duration::seconds(10);
        let rfc2822 = when.to_rfc2822();
        assert_eq!(parse_retry_after(Some(&rfc2822), now), 10);
    }

    #[test]
    fn http_date_in_past_is_zero() {
        let now = fixed_now();
        let past = now - Duration::seconds(5);
        let rfc2822 = past.to_rfc2822();
        assert_eq!(parse_retry_after(Some(&rfc2822), now), 0);
    }

    #[test]
    fn http_date_far_future_capped() {
        let now = fixed_now();
        let far = now + Duration::seconds(3600);
        let rfc2822 = far.to_rfc2822();
        assert_eq!(parse_retry_after(Some(&rfc2822), now), MAX_BACKOFF_SECS);
    }
}
