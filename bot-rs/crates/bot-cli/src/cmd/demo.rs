//! `bot-rs demo` subcommand — Week 1 hackathon demo tick loop.
//!
//! Builds a TickEngine with:
//! - `Venue::Pacifica`    → `PacificaReadOnlyAdapter::production()` (when --pacifica-live)
//! - Other venues         → `DryRunVenueAdapter` loading from --dryrun-fixtures dir
//!
//! Runs a tick loop for each symbol every --tick-interval-secs seconds until
//! --duration-secs elapses or SIGINT is received.

use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use tracing::{info, warn};

use bot_adapters::dryrun::DryRunVenueAdapter;
use bot_adapters::pacifica::PacificaReadOnlyAdapter;
use bot_adapters::pacifica_auth::PacificaAuthenticatedAdapter;
use bot_adapters::venue::VenueAdapter;
use bot_runtime::adapter_health::AdapterHealthRegistry;
use bot_runtime::clock::SimulatedClock;
use bot_runtime::cycle_lock::CycleLockRegistry;
use bot_runtime::history::FundingHistoryRegistry;
use bot_runtime::nav::PortfolioNav;
use bot_runtime::risk::RiskStack;
use bot_runtime::signal::{emit_signal, SignalSections};
use bot_runtime::tick::TickEngine;
use bot_types::Venue;

/// Arguments for the `demo` subcommand.
#[derive(Args, Debug)]
pub struct DemoArgs {
    /// Comma-separated list of symbols to evaluate each tick.
    #[arg(long, default_value = "BTC,ETH,SOL,BNB,ARB,AVAX,SUI,XAU,XAG,PAXG")]
    pub symbols: String,

    /// Total demo duration in seconds. Ignored when `--continuous` is set.
    #[arg(long, default_value_t = 300, conflicts_with = "continuous")]
    pub duration_secs: u64,

    /// Run the tick loop until SIGINT (Ctrl-C) instead of for a fixed
    /// duration. Use during the hackathon demo so the dashboard sees a
    /// continuously-updating `nav.jsonl` for the entire judge window.
    /// Conflicts with `--duration-secs`.
    #[arg(long, default_value_t = false)]
    pub continuous: bool,

    /// Tick interval in seconds.
    #[arg(long, default_value_t = 5)]
    pub tick_interval_secs: u64,

    /// Starting NAV in USD for the simulated tracker.
    #[arg(long, default_value_t = 10_000.0)]
    pub starting_nav: f64,

    /// Connect to live Pacifica REST API (requires network; read-only, no keys).
    #[arg(long, default_value_t = false)]
    pub pacifica_live: bool,

    /// Use the authenticated Pacifica adapter (requires PACIFICA_API_KEY and
    /// PACIFICA_BUILDER_CODE env vars). Enables authenticated account/builder
    /// endpoints and sets `pacifica_authenticated: true` in signal JSON.
    /// Implies --pacifica-live. No order submission.
    #[arg(long, default_value_t = false)]
    pub pacifica_auth: bool,

    /// Path to fixture directory for DryRunVenueAdapters.
    /// Expected layout: {dir}/{venue_name}/{symbol}.json
    #[arg(long)]
    pub dryrun_fixtures: Option<PathBuf>,

    /// Directory for signal JSON output.
    #[arg(long, default_value = "output/signals")]
    pub signal_dir: PathBuf,

    /// Path for NAV JSONL log (one NavPoint per line).
    #[arg(long, default_value = "output/nav.jsonl")]
    pub nav_log: PathBuf,

    /// Simulated-time acceleration factor.
    ///
    /// N=1 (default) = real time. N=3600 = 1 real second == 1 simulated hour.
    /// Applied to NAV accrual, cycle-lock timestamps, and signal JSON `ts_unix`.
    /// NOT applied to live Pacifica API fetches (those use wall clock for
    /// real-time market data).
    #[arg(long, default_value_t = 1.0)]
    pub accel_factor: f64,
}

pub async fn run(args: DemoArgs) -> Result<()> {
    // ── Tracing ───────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // ── Validate accel_factor ─────────────────────────────────────────────
    if args.accel_factor <= 0.0 {
        anyhow::bail!(
            "--accel-factor must be > 0, got {}. Use 1.0 for real-time or 3600.0 \
             for 1 real second = 1 simulated hour.",
            args.accel_factor
        );
    }

    info!("bot-rs demo — Week 1 aurora-omega-1.1.3 hackathon demo");

    // ── Symbols ───────────────────────────────────────────────────────────
    let symbols: Vec<String> = args
        .symbols
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .collect();

    info!(?symbols, "demo symbols");

    // ── Build adapters ────────────────────────────────────────────────────
    let mut adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>> = BTreeMap::new();
    let mut builder_code: Option<String> = None;

    if args.pacifica_auth {
        // Authenticated adapter — wraps the read-only adapter and adds account/builder endpoints.
        // Implies live Pacifica connection. No order submission.
        match PacificaAuthenticatedAdapter::from_env() {
            Ok(adapter) => {
                info!(
                    builder_code = adapter.builder_code(),
                    "pacifica_auth=true — using PacificaAuthenticatedAdapter (authenticated read-only)"
                );
                builder_code = Some(adapter.builder_code().to_string());
                adapters.insert(Venue::Pacifica, Arc::new(adapter));
            }
            Err(e) => {
                anyhow::bail!(
                    "--pacifica-auth requires PACIFICA_API_KEY and PACIFICA_BUILDER_CODE. Error: {e}."
                );
            }
        }
    } else if args.pacifica_live {
        info!(
            "pacifica_live=true — using PacificaReadOnlyAdapter \
             (live REST for all symbols including XAU/XAG/PAXG)"
        );
        adapters.insert(
            Venue::Pacifica,
            Arc::new(PacificaReadOnlyAdapter::production()),
        );
    } else {
        match &args.dryrun_fixtures {
            Some(dir) => {
                info!(
                    fixtures = %dir.display(),
                    "pacifica_live=false — using DryRunVenueAdapter for Pacifica"
                );
                adapters.insert(
                    Venue::Pacifica,
                    Arc::new(DryRunVenueAdapter::new(Venue::Pacifica, dir.clone())),
                );
            }
            None => {
                warn!(
                    "pacifica_live=false and no --dryrun-fixtures provided; \
                     Pacifica adapter skipped. Pass --dryrun-fixtures or --pacifica-live."
                );
            }
        }
    }

    // Dryrun adapters for other venues (if fixtures dir provided).
    for venue in [Venue::Hyperliquid, Venue::Lighter, Venue::Backpack] {
        match &args.dryrun_fixtures {
            Some(dir) => {
                adapters.insert(venue, Arc::new(DryRunVenueAdapter::new(venue, dir.clone())));
            }
            None => {
                warn!(
                    venue = ?venue,
                    "no --dryrun-fixtures provided; venue adapter skipped"
                );
            }
        }
    }

    if adapters.is_empty() {
        anyhow::bail!("No adapters configured. Use --pacifica-live and/or --dryrun-fixtures.");
    }

    // ── Engine + portfolio NAV tracker + cycle-lock registry ──────────────
    let engine = TickEngine::new(adapters, symbols.clone());
    let mut portfolio_nav = PortfolioNav::new(args.starting_nav, &symbols);

    // Runtime risk stack: 6-guard composite (CVaR, kill switch, heartbeat,
    // Pacifica watchdog, venue concentration, drawdown stop).
    // The kill-switch SIGINT handler is armed below.
    let mut risk_stack = RiskStack::new(args.starting_nav);
    let _kill_handler = risk_stack.kill_switch.arm_signal_handler();

    // Funding history registry — per-symbol rolling series for OU/ADF fits.
    let mut history = FundingHistoryRegistry::new();
    let mut cycle_locks = CycleLockRegistry::new();
    let mut adapter_health = AdapterHealthRegistry::new();
    let starting_nav = args.starting_nav;

    // ── Output dirs ───────────────────────────────────────────────────────
    std::fs::create_dir_all(&args.signal_dir)?;
    if let Some(parent) = args.nav_log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut nav_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.nav_log)?;

    // ── Tick loop ─────────────────────────────────────────────────────────
    if args.continuous {
        info!(
            tick_interval_secs = args.tick_interval_secs,
            accel_factor = args.accel_factor,
            starting_nav,
            "starting tick loop in CONTINUOUS mode (until Ctrl-C)"
        );
    } else {
        info!(
            duration_secs = args.duration_secs,
            tick_interval_secs = args.tick_interval_secs,
            accel_factor = args.accel_factor,
            starting_nav,
            "starting tick loop (fixed duration)"
        );
    }

    let start = std::time::Instant::now();
    let duration = std::time::Duration::from_secs(args.duration_secs);
    let tick_interval = std::time::Duration::from_secs(args.tick_interval_secs);
    let continuous = args.continuous;

    // Simulated clock: maps real elapsed time to simulated time.
    let clock = SimulatedClock::new(args.accel_factor);

    // SIGINT handling via a simple flag.
    let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_flag_ctrlc = Arc::clone(&stop_flag);

    // Try to set up SIGINT handler; if it fails (non-Unix), continue without it.
    let _ = ctrlc::set_handler(move || {
        stop_flag_ctrlc.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Track previous tick's simulated timestamp for dt_seconds computation.
    // None on the first tick — we use a synthetic first-tick dt.
    let mut prev_sim_ms: Option<i64> = None;

    while !stop_flag.load(std::sync::atomic::Ordering::SeqCst)
        && (continuous || start.elapsed() < duration)
    {
        let tick_start = std::time::Instant::now();

        // Simulated timestamp for this tick — sampled ONCE, shared by all symbols.
        let now_ms_sim = clock.now_ms();

        // dt_seconds: on the first tick use tick_interval * accel_factor as
        // a synthetic estimate (no prior timestamp to diff against). On
        // subsequent ticks, compute from the actual simulated-ms delta so
        // that clock drift in real sleep resolution is automatically absorbed.
        let dt_seconds_sim = match prev_sim_ms {
            None => args.tick_interval_secs as f64 * args.accel_factor,
            Some(prev) => (now_ms_sim - prev) as f64 / 1000.0,
        };
        prev_sim_ms = Some(now_ms_sim);

        info!(
            real_elapsed_secs = start.elapsed().as_secs_f64(),
            simulated_elapsed_hours = (now_ms_sim - clock.real_start_ms) as f64 / 3_600_000.0,
            accel_factor = args.accel_factor,
            "tick"
        );

        for symbol in &symbols {
            let output = engine
                .run_one_tick(
                    symbol,
                    portfolio_nav.tracker_for(symbol),
                    &mut cycle_locks,
                    &mut adapter_health,
                    &mut risk_stack,
                    &mut history,
                    now_ms_sim,
                    dt_seconds_sim,
                )
                .await?;

            // Write signal BEFORE any adapter submission (§5.3 ordering rule).
            // Use simulated timestamp so downstream readers see simulated time.
            let ts = clock.now_datetime();
            let sections = SignalSections {
                fair_value: &output.fair_value,
                decision: output.decision.as_ref(),
                cycle_lock: &output.cycle_lock,
                nav_after: output.nav_after,
                pacifica_auth: builder_code.as_deref(),
                adapter_health: &output.adapter_health,
                forecast: &output.forecast,
                risk_decision: &output.risk_decision,
                risk_size_multiplier: output.risk_size_multiplier,
            };
            match emit_signal(&args.signal_dir, symbol, ts, sections) {
                Ok(path) => info!(path = %path.display(), "signal written"),
                Err(e) => warn!(error = %e, "signal write failed"),
            }

            // Append per-symbol NAV point to JSONL log.
            let tracker = portfolio_nav.tracker_for(symbol);
            if let Some(last) = tracker.history.last() {
                if let Ok(line) = serde_json::to_string(last) {
                    let _ = writeln!(nav_file, "{}", line);
                }
            }
        }

        // Append AGGREGATE row after all symbols have been processed.
        let agg = portfolio_nav.snapshot_aggregate_point(now_ms_sim);
        if let Ok(line) = serde_json::to_string(&agg) {
            let _ = writeln!(nav_file, "{}", line);
        }

        // Sleep for the remainder of the tick interval.
        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            tokio::time::sleep(tick_interval - elapsed).await;
        }
    }

    // ── Flush and summary ─────────────────────────────────────────────────
    let ended_nav = portfolio_nav.aggregate_nav_usd();
    let pct_change = (ended_nav - starting_nav) / starting_nav * 100.0;

    let real_elapsed_secs = start.elapsed().as_secs_f64();
    let sim_elapsed_ms = clock.now_ms() - clock.real_start_ms;
    let sim_elapsed_hours = sim_elapsed_ms as f64 / 3_600_000.0;

    // Compute per-symbol cumulative accruals for top/bottom ranking.
    // Bps is **NAV-level** (portfolio-wide denominator) so it sums across
    // symbols to the aggregate bps, matching the dashboard labels.
    let mut symbol_accruals: Vec<(String, f64, f64)> = symbols
        .iter()
        .map(|sym| {
            let tracker = portfolio_nav.trackers.get(sym).unwrap();
            let bps = if starting_nav > 0.0 {
                tracker.cumulative_accrual_usd / starting_nav * 10_000.0
            } else {
                0.0
            };
            (sym.clone(), tracker.cumulative_accrual_usd, bps)
        })
        .collect();

    // Sort descending by bps.
    symbol_accruals.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let symbols_label = symbols.join(" ");

    let top3: Vec<String> = symbol_accruals
        .iter()
        .take(3)
        .map(|(sym, _, bps)| format!("    {:6}: {:+.1} bps", sym, bps))
        .collect();

    let bottom3: Vec<String> = symbol_accruals
        .iter()
        .rev()
        .take(3)
        .map(|(sym, _, bps)| format!("    {:6}: {:+.1} bps", sym, bps))
        .collect();

    info!(
        started_nav = starting_nav,
        ended_nav,
        pct_change,
        real_elapsed_secs,
        sim_elapsed_hours,
        accel_factor = args.accel_factor,
        "demo complete"
    );

    println!(
        "\n=== Demo summary ===\
         \n  Symbols:            {}\
         \n  Starting NAV:       ${:.2}\
         \n  Ending NAV:         ${:.2}\
         \n  Change:             {:+.4}%\
         \n  Real elapsed:       {:.1} s\
         \n  Simulated elapsed:  {:.2} h  (accel={:.0})\
         \n  Top 3 symbols:\
         \n{}\
         \n  Bottom 3 symbols:\
         \n{}\n",
        symbols_label,
        starting_nav,
        ended_nav,
        pct_change,
        real_elapsed_secs,
        sim_elapsed_hours,
        args.accel_factor,
        top3.join("\n"),
        bottom3.join("\n"),
    );

    Ok(())
}
