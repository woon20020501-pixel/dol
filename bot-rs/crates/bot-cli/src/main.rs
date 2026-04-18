//! bot-rs CLI entry point.
//!
//! M1 scaffold extended with a `demo` subcommand in Step B.
//! All original M1-M4 functionality is preserved unchanged.

mod cmd;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// bot-rs — Dol v3.5.2 cross-venue funding hedge bot.
#[derive(Parser)]
#[command(name = "bot-rs", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the Week 1 hackathon demo: tick loop with fixture adapters.
    Demo(cmd::demo::DemoArgs),
}

// ── Async runtime configuration ──────────────────────────────────────────────
//
// `flavor = "multi_thread"` is required because the tick engine issues
// `futures_util::future::join_all` on up to 4 venue adapters in parallel,
// each awaiting a network round-trip. A single-threaded runtime would
// serialize those and multiply p99 tick latency by ~4×.
//
// `worker_threads = 4` matches the 4-venue fan-out: Pacifica, Hyperliquid,
// Lighter, Backpack. One OS thread per venue keeps adapter CPU bounded
// (each is ~99 % I/O wait). Operators with heavier universes can override
// via `TOKIO_WORKER_THREADS=<n>` at process start (tokio honors the env
// var when set).
//
// Benchmark evidence (criterion `decision_bench`): decision path is
// 234 ns warm / 587 ns cold — no thread count above 4 gave measurable
// throughput improvement because the hot path is not CPU-bound.
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    // Preflight: if RUNNER_ALLOW_LIVE=1 is set, verify v0 components are
    // wired before starting any subcommand. Otherwise (demo mode) pass
    // silently. See `bot_runtime::live_gate`.
    if let Err(msg) = bot_runtime::live_gate::preflight_live_gate() {
        eprintln!("preflight failed: {msg}");
        std::process::exit(2);
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Demo(args)) => cmd::demo::run(args).await,
        None => {
            // Legacy M1 scaffold behaviour when no subcommand is given.
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .json()
                .init();
            tracing::info!(
                "bot-rs: M1 scaffold — nothing wired yet. \
                 Use `bot-rs demo --help` for the Week 1 hackathon demo."
            );
            Ok(())
        }
    }
}
