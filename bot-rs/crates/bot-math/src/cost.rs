//! D.10 — Model C round-trip execution cost and slippage.
//!
//! v4 Part 1 (Model C hybrid):
//!   c^C = (φ_m^{v_p} + φ_t^{v_p}) + 2·φ_t^{v_c}
//!       + σ(n, Π_p) + 2·σ(n, Π_c)
//!       + 2·ε_leg + β
//!
//! Python reference: `cost_model.slippage` and `round_trip_cost_pct` (adapted
//! for Model C pivot-maker / counter-taker assumption).

use bot_types::{Dimensionless, LiveInputs, PairId, Usd, Venue};

// ---------------------------------------------------------------------------
// Slippage constants (Python cost_model.py reference)
// Calibrated conservative defaults; must be re-calibrated from Phase 1 dry-run fills.
// ---------------------------------------------------------------------------
const KAPPA: f64 = 0.0008; // √-impact coefficient (Almgren-Chriss class)
const DEPTH_OI_FRAC: f64 = 0.10; // 10% of OI treated as "easily reachable" depth
const DEPTH_VOL_FRAC: f64 = 0.01; // 1% of 24h volume treated as "easily reachable" depth
const FLOOR: f64 = 0.0001; // 1 bp floor (always pay tick spread)
const CEILING: f64 = 0.02; // 200 bp ceiling (above this = uncrossable)

/// Square-root market-impact slippage estimator (Almgren-Chriss style).
///
/// # Formula
/// depth = max(DEPTH_OI_FRAC × OI, DEPTH_VOL_FRAC × vol_24h, 1000)
/// raw   = KAPPA × √(n / depth)
/// slip  = clamp(raw, FLOOR, CEILING)
///
/// Returns 0 when `n` ≤ 0.
pub fn slippage(n: Usd, oi: Usd, vol_24h: Usd) -> Dimensionless {
    debug_assert!(n.0 >= 0.0, "slippage: negative notional {}", n.0);
    if n.0 <= 0.0 {
        return Dimensionless(0.0);
    }
    let depth = (DEPTH_OI_FRAC * oi.0)
        .max(DEPTH_VOL_FRAC * vol_24h.0)
        .max(1_000.0);
    let raw = KAPPA * (n.0 / depth).sqrt();
    Dimensionless(raw.clamp(FLOOR, CEILING))
}

/// Model C hybrid round-trip cost (as fraction of single-leg notional).
///
/// # Formula
/// c^C = (φ_m^{v_p} + φ_t^{v_p})   — pivot fees (maker open, taker close or taker both)
///     + 2 · φ_t^{v_c}              — counter fees (taker open + taker close)
///     + σ(n, Π_p)                  — pivot slippage (1×, maker fill assumed 50%)
///     + 2 · σ(n, Π_c)              — counter slippage (2×)
///     + 2 · ε_leg                  — legging drift (open + close)
///     + β                          — bridge round-trip cost
///
/// Fee fallbacks if key missing from `inputs`:
///   pivot maker: 0.00015, pivot taker: 0.00040, counter taker: 0.00050.
///
/// # Arguments
/// - `legging_window_seconds`    — time window (seconds) for simultaneous leg execution
/// - `sigma_price_per_sqrt_day`  — daily price volatility (fraction per √day)
///
/// # Note
/// Sign convention: D > 0 means short-counter receives net funding.
/// This function is sign-agnostic; the caller passes the correct |notional|.
pub fn round_trip_cost_model_c(
    pair: &PairId,
    n_per_leg: Usd,
    inputs: &LiveInputs,
    legging_window_seconds: f64,
    sigma_price_per_sqrt_day: f64,
) -> Dimensionless {
    let v_p = Venue::Pacifica;
    let v_c = pair.counter;

    // --- Fees ---
    // Pivot: assumed maker-open + taker-close (conservative avg: both sides counted)
    let phi_m_p = inputs
        .fee_maker
        .get(&v_p)
        .copied()
        .unwrap_or(Dimensionless(0.00015));
    let phi_t_p = inputs
        .fee_taker
        .get(&v_p)
        .copied()
        .unwrap_or(Dimensionless(0.00040));
    // Counter: taker for both open and close
    let phi_t_c = inputs
        .fee_taker
        .get(&v_c)
        .copied()
        .unwrap_or(Dimensionless(0.00050));

    let fee = (phi_m_p.0 + phi_t_p.0) + 2.0 * phi_t_c.0;

    // --- Slippage ---
    // Pivot side: assumed filled as maker (1× slippage for residual market impact)
    let slip_p = slippage(
        n_per_leg,
        inputs
            .open_interest
            .get(&(pair.symbol.clone(), v_p))
            .copied()
            .unwrap_or(Usd(0.0)),
        inputs
            .volume_24h
            .get(&(pair.symbol.clone(), v_p))
            .copied()
            .unwrap_or(Usd(0.0)),
    );
    // Counter side: taker fill (2× for open + close)
    let slip_c = slippage(
        n_per_leg,
        inputs
            .open_interest
            .get(&(pair.symbol.clone(), v_c))
            .copied()
            .unwrap_or(Usd(0.0)),
        inputs
            .volume_24h
            .get(&(pair.symbol.clone(), v_c))
            .copied()
            .unwrap_or(Usd(0.0)),
    );

    let slip = slip_p.0 + 2.0 * slip_c.0;

    // --- Legging drift ---
    // ε_leg = σ_price × √t_leg / √(2π)
    // σ_price_per_sqrt_day → σ_price_per_sqrt_sec
    let sigma_per_sqrt_sec = sigma_price_per_sqrt_day / (86_400.0_f64).sqrt();
    let epsilon_leg =
        sigma_per_sqrt_sec * legging_window_seconds.sqrt() / (2.0 * std::f64::consts::PI).sqrt();
    // Round trip: open legging + close legging
    let legging = 2.0 * epsilon_leg;

    // --- Bridge ---
    let bridge = inputs
        .bridge_cost
        .get(&(v_p, v_c))
        .copied()
        .unwrap_or(Dimensionless(0.0));

    Dimensionless(fee + slip + legging + bridge.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_types::{AnnualizedRate, LiveInputs, Usd, Venue};
    use hashbrown::HashMap;

    fn empty_inputs() -> LiveInputs {
        LiveInputs {
            timestamp_ms: 0,
            aum: Usd(1_000_000.0),
            r_idle: AnnualizedRate(0.04),
            funding_rate_h: HashMap::new(),
            open_interest: HashMap::new(),
            volume_24h: HashMap::new(),
            fee_maker: HashMap::new(),
            fee_taker: HashMap::new(),
            bridge_cost: HashMap::new(),
            funding_history: HashMap::new(),
            basis_divergence_history: HashMap::new(),
        }
    }

    #[test]
    fn slippage_zero_notional() {
        let s = slippage(Usd(0.0), Usd(1_000_000.0), Usd(5_000_000.0));
        assert_eq!(s.0, 0.0);
    }

    #[test]
    fn slippage_floor_enforced() {
        // Very small n relative to deep market → should hit floor
        let s = slippage(Usd(1.0), Usd(100_000_000.0), Usd(500_000_000.0));
        assert!((s.0 - FLOOR).abs() < 1e-15);
    }

    #[test]
    fn slippage_ceiling_enforced() {
        // Very large n relative to tiny market → should hit ceiling
        let s = slippage(Usd(100_000_000.0), Usd(1_000.0), Usd(0.0));
        assert!((s.0 - CEILING).abs() < 1e-15);
    }

    #[test]
    fn slippage_sqrt_scaling() {
        // Choose n values that produce raw slippage clearly in (FLOOR, CEILING).
        // depth = max(0.10 * 10_000, 0.01 * 100_000, 1_000) = max(1000, 1000, 1000) = 1000
        // n1=1_000: raw = 0.0008 * sqrt(1000/1000) = 0.0008 → above FLOOR=0.0001 ✓
        // n2=4_000: raw = 0.0008 * sqrt(4000/1000) = 0.0008 * 2 = 0.0016 → below CEILING ✓
        // ratio = 2.0 exactly
        let n1 = Usd(1_000.0);
        let n2 = Usd(4_000.0);
        let oi = Usd(10_000.0); // 0.10 * 10_000 = 1_000 = depth
        let vol = Usd(100_000.0); // 0.01 * 100_000 = 1_000 = depth (same)
        let s1 = slippage(n1, oi, vol);
        let s2 = slippage(n2, oi, vol);
        // s2/s1 = sqrt(4000/1000) = 2.0 (both unclamped)
        assert!((s2.0 / s1.0 - 2.0).abs() < 1e-10);
    }

    #[test]
    fn round_trip_cost_fallback_fees() {
        // With empty inputs: fees = (0.00015 + 0.00040) + 2*0.00050 = 0.00155
        // slip_p = slippage(1000, 0, 0) → depth = max(0, 0, 1000) = 1000
        //        = clamp(0.0008 * sqrt(1), FLOOR, CEIL) = 0.0008
        // slip_c = same = 0.0008
        // slip total = 0.0008 + 2*0.0008 = 0.0024
        // legging = 2 * (sigma_per_sqrt_sec * sqrt(t) / sqrt(2π))
        // bridge = 0
        let inputs = empty_inputs();
        let pair = PairId::new("BTC", Venue::Hyperliquid);
        let n = Usd(1_000.0);
        let cost = round_trip_cost_model_c(&pair, n, &inputs, 5.0, 0.02);

        // fees
        let expected_fees = (0.00015 + 0.00040) + 2.0 * 0.00050;
        // slippage — depth=1000, both markets have 0 OI and vol
        let depth = 1_000.0_f64;
        let raw_slip = KAPPA * (1_000.0_f64 / depth).sqrt();
        let slip_each = raw_slip.clamp(FLOOR, CEILING);
        let expected_slip = slip_each + 2.0 * slip_each;
        // legging
        let sigma_per_sqrt_sec = 0.02 / (86_400.0_f64).sqrt();
        let eps = sigma_per_sqrt_sec * 5.0_f64.sqrt() / (2.0 * std::f64::consts::PI).sqrt();
        let expected_legging = 2.0 * eps;

        let expected = expected_fees + expected_slip + expected_legging;
        assert!(
            (cost.0 - expected).abs() < 1e-12,
            "round_trip_cost mismatch: got {}, expected {}",
            cost.0,
            expected
        );
    }

    #[test]
    fn round_trip_cost_is_positive() {
        let inputs = empty_inputs();
        let pair = PairId::new("ETH", Venue::Lighter);
        let cost = round_trip_cost_model_c(&pair, Usd(50_000.0), &inputs, 5.0, 0.02);
        assert!(cost.0 > 0.0);
    }
}
