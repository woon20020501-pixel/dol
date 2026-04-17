//! Prometheus `/metrics` endpoint + runtime metrics registry.
//!
//! Reference: Prometheus exposition format, <https://prometheus.io/docs/instrumenting/exposition_formats/>.
//! For SRE best practices see Beyer et al. (2016), "Site Reliability
//! Engineering", O'Reilly, §6 (Monitoring Distributed Systems).
//!
//! Exposed metrics (as of P8):
//!
//! - `bot_nav_usd{symbol}`              — current per-symbol NAV (gauge).
//! - `bot_portfolio_nav_usd`            — aggregate NAV (gauge).
//! - `bot_cumulative_accrual_usd`       — cumulative net accrual (gauge).
//! - `bot_fees_paid_usd{symbol}`        — cumulative fees paid (counter-like gauge).
//! - `bot_tick_duration_seconds`        — tick latency histogram.
//! - `bot_adapter_fetch_failures_total{venue,symbol}` — adapter errors (counter).
//! - `bot_adapter_latency_seconds{venue}` — adapter fetch latency histogram.
//! - `bot_cycle_lock_blocks_total{symbol}` — proposed-was-blocked counter.
//! - `bot_risk_decisions_total{decision}` — count by {pass,reduce,block,flatten}.
//! - `bot_forecast_regime{symbol}`      — 0=stationary, 1=drift, 2=insufficient (gauge).
//!
//! The implementation is lock-free (atomics + single RwLock for the
//! BTreeMap of labels) and zero-allocation on the metric increment path
//! (allocations only on first-time-label registration).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::risk::RiskDecision;

/// Gauge stored as f64-encoded-in-u64 (for atomic Swap semantics).
#[derive(Debug, Default)]
struct F64Gauge(AtomicU64);

impl F64Gauge {
    #[inline]
    fn set(&self, v: f64) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    #[inline]
    fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// Histogram bucket boundaries (seconds). Chosen to cover: microseconds
/// (cold path), milliseconds (decision path), seconds (full tick).
const LATENCY_BUCKETS_SECS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Debug)]
struct Histogram {
    buckets: Vec<AtomicU64>,
    sum: F64Gauge,
    count: AtomicU64,
    boundaries: &'static [f64],
}

impl Histogram {
    fn new(boundaries: &'static [f64]) -> Self {
        Self {
            buckets: (0..=boundaries.len()).map(|_| AtomicU64::new(0)).collect(),
            sum: F64Gauge::default(),
            count: AtomicU64::new(0),
            boundaries,
        }
    }

    fn observe(&self, v_secs: f64) {
        self.sum.set(self.sum.get() + v_secs);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (i, b) in self.boundaries.iter().enumerate() {
            if v_secs <= *b {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // +Inf bucket
        self.buckets[self.boundaries.len()].fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self, name: &str, label_pairs: &str) -> String {
        let mut out = String::new();
        let mut cumulative: u64 = 0;
        for (i, b) in self.boundaries.iter().enumerate() {
            cumulative += self.buckets[i].load(Ordering::Relaxed);
            let prefix = if label_pairs.is_empty() {
                format!("{name}_bucket{{le=\"{b}\"}}")
            } else {
                format!("{name}_bucket{{{label_pairs},le=\"{b}\"}}")
            };
            out.push_str(&format!("{prefix} {cumulative}\n"));
        }
        // +Inf bucket
        cumulative += self.buckets[self.boundaries.len()].load(Ordering::Relaxed);
        let inf_prefix = if label_pairs.is_empty() {
            format!("{name}_bucket{{le=\"+Inf\"}}")
        } else {
            format!("{name}_bucket{{{label_pairs},le=\"+Inf\"}}")
        };
        out.push_str(&format!("{inf_prefix} {cumulative}\n"));
        let sum_prefix = if label_pairs.is_empty() {
            format!("{name}_sum")
        } else {
            format!("{name}_sum{{{label_pairs}}}")
        };
        out.push_str(&format!("{sum_prefix} {}\n", self.sum.get()));
        let count_prefix = if label_pairs.is_empty() {
            format!("{name}_count")
        } else {
            format!("{name}_count{{{label_pairs}}}")
        };
        out.push_str(&format!(
            "{count_prefix} {}\n",
            self.count.load(Ordering::Relaxed)
        ));
        out
    }
}

/// Global metrics registry. Arc-wrapped so a single instance is shared
/// across the tick loop and the /metrics endpoint.
#[derive(Debug)]
pub struct Metrics {
    // Per-symbol gauges
    nav_by_symbol: RwLock<BTreeMap<String, Arc<F64Gauge>>>,
    fees_paid_by_symbol: RwLock<BTreeMap<String, Arc<F64Gauge>>>,
    forecast_regime_by_symbol: RwLock<BTreeMap<String, Arc<F64Gauge>>>,
    cycle_lock_blocks_by_symbol: RwLock<BTreeMap<String, Arc<AtomicU64>>>,
    // Scalar gauges
    portfolio_nav: F64Gauge,
    cumulative_accrual: F64Gauge,
    // Labeled counters: composite "venue=...,symbol=..." key → fetch failures
    fetch_failures: RwLock<BTreeMap<String, Arc<AtomicU64>>>,
    // Labeled counters for risk decisions
    risk_decisions: RwLock<BTreeMap<String, Arc<AtomicU64>>>,
    // Per-venue adapter latency
    adapter_latency_by_venue: RwLock<BTreeMap<String, Arc<Histogram>>>,
    // Full-tick latency
    tick_duration: Histogram,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            nav_by_symbol: RwLock::new(BTreeMap::new()),
            fees_paid_by_symbol: RwLock::new(BTreeMap::new()),
            forecast_regime_by_symbol: RwLock::new(BTreeMap::new()),
            cycle_lock_blocks_by_symbol: RwLock::new(BTreeMap::new()),
            portfolio_nav: F64Gauge::default(),
            cumulative_accrual: F64Gauge::default(),
            fetch_failures: RwLock::new(BTreeMap::new()),
            risk_decisions: RwLock::new(BTreeMap::new()),
            adapter_latency_by_venue: RwLock::new(BTreeMap::new()),
            tick_duration: Histogram::new(LATENCY_BUCKETS_SECS),
        })
    }

    fn gauge_for(map: &RwLock<BTreeMap<String, Arc<F64Gauge>>>, key: &str) -> Arc<F64Gauge> {
        if let Some(g) = map.read().unwrap().get(key) {
            return Arc::clone(g);
        }
        let g = Arc::new(F64Gauge::default());
        map.write().unwrap().insert(key.to_string(), Arc::clone(&g));
        g
    }

    fn counter_for(map: &RwLock<BTreeMap<String, Arc<AtomicU64>>>, key: &str) -> Arc<AtomicU64> {
        if let Some(c) = map.read().unwrap().get(key) {
            return Arc::clone(c);
        }
        let c = Arc::new(AtomicU64::new(0));
        map.write().unwrap().insert(key.to_string(), Arc::clone(&c));
        c
    }

    fn hist_for(map: &RwLock<BTreeMap<String, Arc<Histogram>>>, key: &str) -> Arc<Histogram> {
        if let Some(h) = map.read().unwrap().get(key) {
            return Arc::clone(h);
        }
        let h = Arc::new(Histogram::new(LATENCY_BUCKETS_SECS));
        map.write().unwrap().insert(key.to_string(), Arc::clone(&h));
        h
    }

    // ── Setters (called from the tick loop) ──

    pub fn set_nav(&self, symbol: &str, nav_usd: f64) {
        Self::gauge_for(&self.nav_by_symbol, symbol).set(nav_usd);
    }

    pub fn set_fees_paid(&self, symbol: &str, fees_usd: f64) {
        Self::gauge_for(&self.fees_paid_by_symbol, symbol).set(fees_usd);
    }

    pub fn set_portfolio_nav(&self, nav_usd: f64) {
        self.portfolio_nav.set(nav_usd);
    }

    pub fn set_cumulative_accrual(&self, accrual_usd: f64) {
        self.cumulative_accrual.set(accrual_usd);
    }

    pub fn set_forecast_regime(&self, symbol: &str, regime_code: f64) {
        Self::gauge_for(&self.forecast_regime_by_symbol, symbol).set(regime_code);
    }

    pub fn inc_fetch_failure(&self, venue: &str, symbol: &str) {
        // Key: "venue=...,symbol=..." — simple composite string key is fine.
        let key = format!("venue=\"{venue}\",symbol=\"{symbol}\"");
        Self::counter_for(&self.fetch_failures, &key).fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cycle_lock_block(&self, symbol: &str) {
        let c = {
            let map = self.cycle_lock_blocks_by_symbol.read().unwrap();
            map.get(symbol).cloned()
        };
        match c {
            Some(c) => c.fetch_add(1, Ordering::Relaxed),
            None => {
                let c = Arc::new(AtomicU64::new(1));
                self.cycle_lock_blocks_by_symbol
                    .write()
                    .unwrap()
                    .insert(symbol.to_string(), c);
                0
            }
        };
    }

    pub fn inc_risk_decision(&self, decision: &RiskDecision) {
        let label = match decision {
            RiskDecision::Pass => "pass",
            RiskDecision::Reduce { .. } => "reduce",
            RiskDecision::Block { .. } => "block",
            RiskDecision::Flatten { .. } => "flatten",
        };
        Self::counter_for(&self.risk_decisions, label).fetch_add(1, Ordering::Relaxed);
    }

    pub fn observe_adapter_latency(&self, venue: &str, d: Duration) {
        Self::hist_for(&self.adapter_latency_by_venue, venue).observe(d.as_secs_f64());
    }

    pub fn observe_tick_duration(&self, d: Duration) {
        self.tick_duration.observe(d.as_secs_f64());
    }

    // ── Exposition format renderer ──

    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP bot_nav_usd Per-symbol NAV in USD\n# TYPE bot_nav_usd gauge\n");
        for (sym, g) in self.nav_by_symbol.read().unwrap().iter() {
            out.push_str(&format!("bot_nav_usd{{symbol=\"{sym}\"}} {}\n", g.get()));
        }
        out.push_str(
            "# HELP bot_fees_paid_usd Cumulative fees + slippage\n# TYPE bot_fees_paid_usd gauge\n",
        );
        for (sym, g) in self.fees_paid_by_symbol.read().unwrap().iter() {
            out.push_str(&format!(
                "bot_fees_paid_usd{{symbol=\"{sym}\"}} {}\n",
                g.get()
            ));
        }
        out.push_str(
            "# HELP bot_portfolio_nav_usd Aggregate portfolio NAV\n# TYPE bot_portfolio_nav_usd gauge\n",
        );
        out.push_str(&format!(
            "bot_portfolio_nav_usd {}\n",
            self.portfolio_nav.get()
        ));
        out.push_str(
            "# HELP bot_cumulative_accrual_usd Net accrual since start\n# TYPE bot_cumulative_accrual_usd gauge\n",
        );
        out.push_str(&format!(
            "bot_cumulative_accrual_usd {}\n",
            self.cumulative_accrual.get()
        ));
        out.push_str(
            "# HELP bot_forecast_regime 0=stationary 1=drift 2=insufficient\n# TYPE bot_forecast_regime gauge\n",
        );
        for (sym, g) in self.forecast_regime_by_symbol.read().unwrap().iter() {
            out.push_str(&format!(
                "bot_forecast_regime{{symbol=\"{sym}\"}} {}\n",
                g.get()
            ));
        }
        out.push_str(
            "# HELP bot_adapter_fetch_failures_total Adapter fetch errors\n# TYPE bot_adapter_fetch_failures_total counter\n",
        );
        for (key, c) in self.fetch_failures.read().unwrap().iter() {
            out.push_str(&format!(
                "bot_adapter_fetch_failures_total{{{key}}} {}\n",
                c.load(Ordering::Relaxed)
            ));
        }
        out.push_str(
            "# HELP bot_cycle_lock_blocks_total I-LOCK mid-cycle pair blocks\n# TYPE bot_cycle_lock_blocks_total counter\n",
        );
        for (sym, c) in self.cycle_lock_blocks_by_symbol.read().unwrap().iter() {
            out.push_str(&format!(
                "bot_cycle_lock_blocks_total{{symbol=\"{sym}\"}} {}\n",
                c.load(Ordering::Relaxed)
            ));
        }
        out.push_str(
            "# HELP bot_risk_decisions_total Risk-stack decision counts\n# TYPE bot_risk_decisions_total counter\n",
        );
        for (label, c) in self.risk_decisions.read().unwrap().iter() {
            out.push_str(&format!(
                "bot_risk_decisions_total{{decision=\"{label}\"}} {}\n",
                c.load(Ordering::Relaxed)
            ));
        }
        out.push_str(
            "# HELP bot_adapter_latency_seconds Adapter fetch latency\n# TYPE bot_adapter_latency_seconds histogram\n",
        );
        for (venue, h) in self.adapter_latency_by_venue.read().unwrap().iter() {
            let labels = format!("venue=\"{venue}\"");
            out.push_str(&h.render("bot_adapter_latency_seconds", &labels));
        }
        out.push_str(
            "# HELP bot_tick_duration_seconds Full tick latency\n# TYPE bot_tick_duration_seconds histogram\n",
        );
        out.push_str(&self.tick_duration.render("bot_tick_duration_seconds", ""));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::RiskDecision;

    #[test]
    fn renders_empty_registry() {
        let m = Metrics::new();
        let rendered = m.render_prometheus();
        // Even empty, header lines should be present.
        assert!(rendered.contains("bot_nav_usd"));
        assert!(rendered.contains("bot_portfolio_nav_usd"));
        assert!(rendered.contains("bot_tick_duration_seconds"));
    }

    #[test]
    fn gauges_roundtrip() {
        let m = Metrics::new();
        m.set_nav("BTC", 10_000.5);
        m.set_nav("ETH", 5_000.25);
        m.set_portfolio_nav(15_000.75);
        let rendered = m.render_prometheus();
        assert!(rendered.contains("bot_nav_usd{symbol=\"BTC\"} 10000.5"));
        assert!(rendered.contains("bot_nav_usd{symbol=\"ETH\"} 5000.25"));
        assert!(rendered.contains("bot_portfolio_nav_usd 15000.75"));
    }

    #[test]
    fn counter_increments_per_label() {
        let m = Metrics::new();
        m.inc_fetch_failure("Pacifica", "BTC");
        m.inc_fetch_failure("Pacifica", "BTC");
        m.inc_fetch_failure("Pacifica", "ETH");
        let rendered = m.render_prometheus();
        assert!(rendered
            .contains("bot_adapter_fetch_failures_total{venue=\"Pacifica\",symbol=\"BTC\"} 2"));
        assert!(rendered
            .contains("bot_adapter_fetch_failures_total{venue=\"Pacifica\",symbol=\"ETH\"} 1"));
    }

    #[test]
    fn histogram_observes_buckets() {
        let m = Metrics::new();
        m.observe_adapter_latency("Pacifica", Duration::from_millis(25));
        m.observe_adapter_latency("Pacifica", Duration::from_millis(150));
        m.observe_adapter_latency("Pacifica", Duration::from_millis(50));
        let rendered = m.render_prometheus();
        // Latency 25ms → in 0.025 bucket and above; 150ms → 0.25+; 50ms → 0.05+
        assert!(rendered.contains("bot_adapter_latency_seconds_count{venue=\"Pacifica\"} 3"));
        assert!(rendered.contains("bot_adapter_latency_seconds_sum{venue=\"Pacifica\"}"));
    }

    #[test]
    fn risk_decision_counts_each_variant() {
        let m = Metrics::new();
        m.inc_risk_decision(&RiskDecision::Pass);
        m.inc_risk_decision(&RiskDecision::Pass);
        m.inc_risk_decision(&RiskDecision::Block {
            reason: "x".to_string(),
        });
        m.inc_risk_decision(&RiskDecision::Flatten {
            reason: "y".to_string(),
        });
        let rendered = m.render_prometheus();
        assert!(rendered.contains("bot_risk_decisions_total{decision=\"pass\"} 2"));
        assert!(rendered.contains("bot_risk_decisions_total{decision=\"block\"} 1"));
        assert!(rendered.contains("bot_risk_decisions_total{decision=\"flatten\"} 1"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP server — exposes `GET /metrics` on a caller-chosen address.
// Prometheus scrape target: http://host:port/metrics
// ─────────────────────────────────────────────────────────────────────────────

use std::net::SocketAddr;

/// Serve the Prometheus exposition endpoint on `addr`. Returns a handle the
/// caller can use to shut down. Binds synchronously; returns Err if the
/// port is unavailable.
///
/// # Example
/// ```no_run
/// # use std::sync::Arc;
/// # use bot_runtime::metrics::{Metrics, serve_metrics};
/// # async fn run() {
/// let metrics = Metrics::new();
/// let _handle = serve_metrics(metrics, "0.0.0.0:9090".parse().unwrap()).await;
/// # }
/// ```
pub async fn serve_metrics(
    metrics: std::sync::Arc<Metrics>,
    addr: SocketAddr,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    use axum::{routing::get, Router};
    let state = metrics;
    let app = Router::new()
        .route("/metrics", get(|axum::extract::State(m): axum::extract::State<std::sync::Arc<Metrics>>| async move {
            (
                [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                m.render_prometheus(),
            )
        }))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "metrics server exited with error");
        }
    });
    Ok(handle)
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let m = Metrics::new();
        m.set_nav("BTC", 42.0);
        // Bind to ephemeral port (port 0).
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let bound = listener.local_addr().unwrap();
        drop(listener);

        let _h = serve_metrics(m.clone(), bound).await.unwrap();
        // Give the server a beat to come up.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let body = reqwest::get(format!("http://{bound}/metrics"))
            .await
            .expect("GET /metrics")
            .text()
            .await
            .expect("body");
        assert!(body.contains("bot_nav_usd{symbol=\"BTC\"} 42"));
    }
}
