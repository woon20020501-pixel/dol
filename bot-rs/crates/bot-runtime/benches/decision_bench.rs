//! Criterion benchmark for the decision hot path.
//!
//! Measures `decision::decide()` with 4 venues × realistic OI/volume,
//! both cold-start (no held pair) and warm (held pair, no-rebalance path).
//!
//! Run: `cargo bench -p bot-runtime --bench decision_bench`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use bot_adapters::venue::VenueSnapshot;
use bot_runtime::decision::{self, PairDecision};
use bot_types::{AnnualizedRate, HourlyRate, Usd, Venue};

fn make_snap(venue: Venue, funding_annual: f64) -> VenueSnapshot {
    VenueSnapshot {
        venue,
        symbol: "BTC".to_string(),
        ts_ms: 1_776_225_863_000,
        mid_price: 100_000.0,
        bid_price: 99_995.0,
        ask_price: 100_005.0,
        tick_size: 0.1,
        mark_bias_bps: 0.0,
        depth_top_usd: 50_000.0,
        depth_curve: vec![
            (1.0, 50_000.0),
            (2.0, 50_000.0),
            (5.0, 50_000.0),
            (10.0, 50_000.0),
            (20.0, 50_000.0),
        ],
        funding_rate_annual: AnnualizedRate(funding_annual),
        funding_rate_hourly: HourlyRate(funding_annual / (365.0 * 24.0)),
        funding_interval_seconds: 3600,
        next_funding_ts_ms: 1_776_225_863_000 + 3_600_000,
        volume_24h_usd: Usd(1_000_000_000.0),
        open_interest_usd: Usd(500_000_000.0),
    }
}

fn four_venue_snapshots() -> Vec<VenueSnapshot> {
    vec![
        make_snap(Venue::Pacifica, 0.03),
        make_snap(Venue::Hyperliquid, 0.06),
        make_snap(Venue::Lighter, 0.02),
        make_snap(Venue::Backpack, 0.08),
    ]
}

fn bench_decide_cold(c: &mut Criterion) {
    let snaps = four_venue_snapshots();
    c.bench_function("decide_cold_4venues", |b| {
        b.iter(|| {
            black_box(decision::decide(
                black_box(&snaps),
                black_box(100_000.0),
                black_box(0.0002),
                black_box(None),
            ))
        })
    });
}

fn bench_decide_warm(c: &mut Criterion) {
    let snaps = four_venue_snapshots();
    let held = PairDecision {
        long_venue: Venue::Lighter,
        short_venue: Venue::Backpack,
        symbol: "BTC".to_string(),
        spread_annual: 0.06,
        cost_fraction: 0.0015,
        net_annual: 0.0585,
        notional_usd: 1_000.0,
        reason: "prior".to_string(),
        would_have_executed: true,
    };
    c.bench_function("decide_warm_norebalance", |b| {
        b.iter(|| {
            black_box(decision::decide(
                black_box(&snaps),
                black_box(100_000.0),
                black_box(0.0002),
                black_box(Some(&held)),
            ))
        })
    });
}

fn bench_fair_value(c: &mut Criterion) {
    let snaps = four_venue_snapshots();
    c.bench_function("fair_value_4venues", |b| {
        b.iter(|| {
            black_box(bot_runtime::fair_value::compute_weighted_fair_value(
                black_box(&snaps),
            ))
        })
    });
}

fn bench_cycle_lock_enforce(c: &mut Criterion) {
    use bot_runtime::cycle_lock::CycleLockRegistry;
    let d = PairDecision {
        long_venue: Venue::Pacifica,
        short_venue: Venue::Backpack,
        symbol: "BTC".to_string(),
        spread_annual: 0.06,
        cost_fraction: 0.0015,
        net_annual: 0.0585,
        notional_usd: 1_000.0,
        reason: "bench".to_string(),
        would_have_executed: true,
    };

    c.bench_function("cycle_lock_enforce_held", |b| {
        let mut reg = CycleLockRegistry::new();
        // Open a cycle first
        reg.enforce_decision("BTC", 1_700_000_000.0, Some(&d), false);
        b.iter(|| {
            black_box(reg.enforce_decision(
                black_box("BTC"),
                black_box(1_700_000_500.0),
                black_box(Some(&d)),
                black_box(false),
            ))
        })
    });
}

criterion_group!(
    benches,
    bench_decide_cold,
    bench_decide_warm,
    bench_fair_value,
    bench_cycle_lock_enforce,
);
criterion_main!(benches);
