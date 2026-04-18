//! Automated secret redaction for `tracing` field values.
//!
//! Wraps a `tracing_subscriber::fmt::Layer` with a field visitor that
//! scrubs any field whose **name** matches one of the
//! reserved sensitive identifiers. Ensures an operator can never
//! accidentally log `api_key = "abc..."` or `signature = "xyz..."` by
//! referencing the field in an `info!`/`warn!` call.
//!
//! # Reserved sensitive field names
//!
//! Matched case-insensitively against the tracing field key:
//!
//! - `api_key`, `api-key`, `apikey`
//! - `signature`, `sig`
//! - `private_key`, `private-key`, `privatekey`, `priv_key`, `secret_key`,
//!   `secret-key`
//! - `seed`, `mnemonic`
//! - `password`, `passwd`
//! - `authorization`, `bearer`
//!
//! The value is replaced with the constant `"<redacted:N>"` where `N` is the
//! byte length of the original (so logs still show presence vs absence).
//!
//! # Usage
//!
//! ```no_run
//! use bot_runtime::tracing_redact::RedactingFormat;
//! use tracing_subscriber::fmt;
//! let _ = fmt::Subscriber::builder().event_format(RedactingFormat).try_init();
//! tracing::info!(api_key = "abc123", symbol = "BTC", "tick");
//! // → `symbol="BTC" api_key=<redacted:6> tick`
//! ```

use std::fmt;

use tracing::field::{Field, Visit};
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

/// List of field names (case-insensitive) whose values get redacted.
/// Extend this list by PR review — not runtime-configurable on purpose so
/// an operator can't be tricked into leaking via env var.
pub const REDACTED_FIELD_NAMES: &[&str] = &[
    "api_key",
    "api-key",
    "apikey",
    "signature",
    "sig",
    "private_key",
    "private-key",
    "privatekey",
    "priv_key",
    "secret_key",
    "secret-key",
    "seed",
    "mnemonic",
    "password",
    "passwd",
    "authorization",
    "bearer",
];

/// Return `true` iff the field name matches one of the sensitive identifiers.
pub fn is_sensitive_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    REDACTED_FIELD_NAMES.iter().any(|n| *n == lower)
}

/// Redact the value component for a sensitive field.
/// Preserves byte length so operators can see whether the field had content.
pub fn redact_value(raw: &str) -> String {
    format!("<redacted:{}>", raw.len())
}

// ─────────────────────────────────────────────────────────────────────────────
// RedactVisit — a `tracing::field::Visit` impl that rewrites fields as it
// walks them. Used inside the custom formatter below.
// ─────────────────────────────────────────────────────────────────────────────

struct RedactVisit<'a, 'b> {
    writer: &'a mut Writer<'b>,
    first: bool,
}

impl<'a, 'b> RedactVisit<'a, 'b> {
    fn new(writer: &'a mut Writer<'b>) -> Self {
        Self {
            writer,
            first: true,
        }
    }

    fn write_sep(&mut self) -> fmt::Result {
        if self.first {
            self.first = false;
            Ok(())
        } else {
            self.writer.write_str(" ")
        }
    }
}

impl<'a, 'b> Visit for RedactVisit<'a, 'b> {
    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = self.write_sep();
        if is_sensitive_field(field.name()) {
            let _ = write!(self.writer, "{}={}", field.name(), redact_value(value));
        } else {
            let _ = write!(self.writer, "{}={:?}", field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = self.write_sep();
        if is_sensitive_field(field.name()) {
            // For debug-formatted sensitive fields we can't know the byte
            // length cheaply, so emit a fixed sentinel.
            let _ = write!(self.writer, "{}=<redacted>", field.name());
        } else {
            let _ = write!(self.writer, "{}={:?}", field.name(), value);
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        let _ = self.write_sep();
        let _ = write!(self.writer, "{}={}", field.name(), value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        let _ = self.write_sep();
        let _ = write!(self.writer, "{}={}", field.name(), value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        let _ = self.write_sep();
        let _ = write!(self.writer, "{}={}", field.name(), value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let _ = self.write_sep();
        let _ = write!(self.writer, "{}={}", field.name(), value);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatter glue — a minimal FormatEvent that emits redacted fields.
// ─────────────────────────────────────────────────────────────────────────────

pub struct RedactingFormat;

impl<S, N> FormatEvent<S, N> for RedactingFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        write!(
            writer,
            "{} {} {}: ",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            meta.level(),
            meta.target()
        )?;
        let mut visitor = RedactVisit::new(&mut writer);
        event.record(&mut visitor);
        writeln!(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names_match_case_insensitively() {
        assert!(is_sensitive_field("api_key"));
        assert!(is_sensitive_field("API_KEY"));
        assert!(is_sensitive_field("Signature"));
        assert!(is_sensitive_field("PRIVATE_KEY"));
        assert!(is_sensitive_field("seed"));
        assert!(is_sensitive_field("Mnemonic"));
    }

    #[test]
    fn benign_names_not_redacted() {
        assert!(!is_sensitive_field("symbol"));
        assert!(!is_sensitive_field("nav_usd"));
        assert!(!is_sensitive_field("venue"));
        assert!(!is_sensitive_field("cycle_index"));
    }

    #[test]
    fn redact_value_preserves_byte_length() {
        assert_eq!(redact_value("abc"), "<redacted:3>");
        assert_eq!(redact_value(""), "<redacted:0>");
        assert_eq!(redact_value("x".repeat(100).as_str()), "<redacted:100>");
    }

    /// End-to-end: emit a log event via a subscriber using the redaction
    /// formatter and verify the captured output contains `<redacted:N>`
    /// for a sensitive field and the literal value for a benign field.
    #[test]
    fn redaction_roundtrip_via_subscriber() {
        use tracing_subscriber::fmt::MakeWriter;

        // MakeWriter impl that appends to a shared Vec<u8>.
        #[derive(Clone)]
        struct BufWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for BufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for BufWriter {
            type Writer = BufWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = BufWriter(buf.clone());

        let subscriber = tracing_subscriber::fmt()
            .event_format(RedactingFormat)
            .with_writer(writer)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(api_key = "topsecret", symbol = "BTC", "event");
        });

        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            captured.contains("api_key=<redacted:9>"),
            "expected redacted api_key, got: {captured}"
        );
        assert!(
            captured.contains(r#"symbol="BTC""#),
            "expected raw symbol, got: {captured}"
        );
        assert!(
            !captured.contains("topsecret"),
            "secret leaked into output: {captured}"
        );
    }
}
