//! Property-based tests for the math layer.
//!
//! Uses `proptest` (BurntSushi port of QuickCheck) to generate thousands of
//! random inputs per property and assert mathematical invariants hold.
//!
//! Each property has a documented source:
//!   - Slippage monotonicity    : square-root impact, ∂σ/∂n ≥ 0 (Almgren-Chriss 2000).
//!   - φ invariants             : Analytic properties of 1-exp(-x)/x absorption kernel.
//!   - HHI bounds               : Hirschman (1964), HHI ∈ [1/N, 1].
//!   - CVaR monotonicity        : Rockafellar-Uryasev (2000) Theorem 1 — ∂CVaR/∂α ≥ 0.
//!   - cap_routing conservation : PRINCIPLES.md spec §I.1 — ∑ slices = gross.
//!   - Bernstein monotonicity   : L^R(τ) decreasing in τ (more variance → tighter bound).
//!
//! Run with `cargo test -p bot-math --test proptests` (or full workspace).

use proptest::prelude::*;

use bot_math::{
    breakeven::break_even_hold_at_mean,
    cost::slippage,
    cvar::{cvar_empirical, cvar_ru},
    leverage::bernstein_leverage_bound,
    phi::phi,
    routing::{cap_routing, mandate_floor},
};
use bot_types::{AnnualizedRate, Dimensionless, Hours, Mandate, Usd};

// ─────────────────────────────────────────────────────────────────────────────
// φ (absorption function) invariants
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// φ(x) ∈ (0, 1] for all finite x ≥ 0.
    /// Ref: φ(x) = (1 - exp(-x))/x; positive on ℝ≥0; φ(0) = 1 (Taylor limit).
    #[test]
    fn phi_is_in_unit_interval(x in 0.0f64..1e6) {
        let v = phi(x);
        prop_assert!(v > 0.0, "φ({}) = {} must be positive", x, v);
        prop_assert!(v <= 1.0 + 1e-12, "φ({}) = {} must be ≤ 1", x, v);
        prop_assert!(v.is_finite(), "φ({}) = {} must be finite", x, v);
    }

    /// φ(x) is monotonically DECREASING in x (for x > 0).
    /// d/dx φ(x) = [exp(-x)(x+1) - 1] / x² ≤ 0 for x ≥ 0.
    #[test]
    fn phi_is_monotone_decreasing(x in 0.01f64..1e3, delta in 0.001f64..10.0) {
        let a = phi(x);
        let b = phi(x + delta);
        prop_assert!(
            b <= a + 1e-12,
            "φ({}) = {} should be ≥ φ({}) = {}",
            x, a, x + delta, b
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slippage invariants (Almgren-Chriss 2000, §4 temporary impact)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// Slippage is monotone non-decreasing in notional.
    /// Formula: σ = κ √(n/depth) clamped to [FLOOR, CEILING].
    /// ∂σ/∂n ≥ 0 (square-root function).
    #[test]
    fn slippage_monotone_in_notional(
        oi in 1_000_000.0f64..1e9,
        vol in 10_000_000.0f64..1e10,
        n_small in 100.0f64..1e5,
        delta_n in 1.0f64..1e5,
    ) {
        let s1 = slippage(Usd(n_small), Usd(oi), Usd(vol)).0;
        let s2 = slippage(Usd(n_small + delta_n), Usd(oi), Usd(vol)).0;
        prop_assert!(
            s2 >= s1 - 1e-15,
            "σ should be non-decreasing: σ({}) = {} vs σ({}) = {}",
            n_small, s1, n_small + delta_n, s2
        );
    }

    /// Slippage always in [0, CEILING=0.02].
    #[test]
    fn slippage_in_range(
        n in 0.0f64..1e10,
        oi in 0.0f64..1e12,
        vol in 0.0f64..1e12,
    ) {
        let s = slippage(Usd(n), Usd(oi), Usd(vol)).0;
        prop_assert!(s >= 0.0, "slippage must be non-negative, got {}", s);
        prop_assert!(s <= 0.02 + 1e-15, "slippage must be ≤ CEILING, got {}", s);
    }

    /// Slippage is monotone non-INcreasing in depth (OI).
    /// As depth grows, the same notional has smaller impact.
    #[test]
    fn slippage_decreasing_in_depth(
        n in 10_000.0f64..1e6,
        oi_small in 10_000.0f64..1e6,
        oi_delta in 10_000.0f64..1e9,
    ) {
        let vol = 1e9_f64; // fixed high volume so OI dominates depth
        let s1 = slippage(Usd(n), Usd(oi_small), Usd(vol)).0;
        let s2 = slippage(Usd(n), Usd(oi_small + oi_delta), Usd(vol)).0;
        prop_assert!(
            s2 <= s1 + 1e-15,
            "larger depth must not increase slippage: σ(oi={}) = {} vs σ(oi={}) = {}",
            oi_small, s1, oi_small + oi_delta, s2
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// cap_routing conservation (PRINCIPLES.md spec §I.1)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// ∑ slices == vault_gross within f64 epsilon × gross.
    #[test]
    fn cap_routing_conserves_gross(gross in 0.0f64..1.0) {
        let m = Mandate::default();
        let alloc = cap_routing(AnnualizedRate(gross), &m);
        let sum = alloc.customer.0 + alloc.buffer.0 + alloc.reserve.0;
        prop_assert!(
            (sum - gross).abs() < 1e-12 + gross.abs() * 1e-12,
            "conservation violated at gross={}: sum={} diff={}",
            gross, sum, sum - gross
        );
    }

    /// Each slice never exceeds its cap.
    #[test]
    fn cap_routing_respects_caps(gross in 0.0f64..2.0) {
        let m = Mandate::default();
        let alloc = cap_routing(AnnualizedRate(gross), &m);
        prop_assert!(
            alloc.customer.0 <= m.customer_apy_max.0 + 1e-12,
            "customer cap violated at gross={}: {} > {}",
            gross, alloc.customer.0, m.customer_apy_max.0
        );
        prop_assert!(
            alloc.buffer.0 <= m.buffer_apy_max.0 + 1e-12,
            "buffer cap violated at gross={}: {} > {}",
            gross, alloc.buffer.0, m.buffer_apy_max.0
        );
        prop_assert!(
            alloc.reserve.0 >= -1e-12,
            "reserve should be non-negative, got {}",
            alloc.reserve.0
        );
    }

    /// mandate_floor is non-negative.
    #[test]
    fn mandate_floor_non_negative(_noop in 0u8..1) {
        let m = Mandate::default();
        let floor = mandate_floor(&m);
        prop_assert!(floor.0 >= 0.0, "mandate_floor = {} < 0", floor.0);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CVaR invariants (Rockafellar-Uryasev 2000)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// CVaR monotone non-decreasing in α: α1 ≤ α2 ⇒ CVaR_α1 ≤ CVaR_α2.
    /// Ref: Theorem 1 in Rockafellar-Uryasev.
    #[test]
    fn cvar_monotone_in_alpha(
        samples in prop::collection::vec(0.0f64..1000.0, 100..500),
        a1 in 0.1f64..0.5,
        a2_delta in 0.05f64..0.4,
    ) {
        let a2 = (a1 + a2_delta).min(0.99);
        let c1 = cvar_empirical(&samples, a1).unwrap_or(f64::NEG_INFINITY);
        let c2 = cvar_empirical(&samples, a2).unwrap_or(f64::NEG_INFINITY);
        prop_assert!(
            c2 >= c1 - 1e-9,
            "CVaR_{} = {} must be ≥ CVaR_{} = {}",
            a2, c2, a1, c1
        );
    }

    /// CVaR_α ≥ VaR_α (the tail conditional expectation is above the quantile).
    #[test]
    fn cvar_above_var(
        samples in prop::collection::vec(-100.0f64..100.0, 100..300),
        alpha in 0.5f64..0.99,
    ) {
        let (cvar, var) = cvar_ru(&samples, alpha).unwrap();
        prop_assert!(
            cvar >= var - 1e-9,
            "CVaR = {} must be ≥ VaR = {} at α = {}",
            cvar, var, alpha
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bernstein leverage bound (monotone in τ)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// L^R(τ) is monotone non-increasing in τ (longer hold → more variance → tighter bound).
    #[test]
    fn bernstein_leverage_monotone_in_tau(
        mmr in 0.01f64..0.20,
        delta in 0.0001f64..0.05,
        sigma in 0.0001f64..0.05,
        tau_small in 1.0f64..200.0,
        tau_delta in 1.0f64..500.0,
        eps in 0.001f64..0.10,
    ) {
        let tau_big = tau_small + tau_delta;
        let l1 = bernstein_leverage_bound(
            Dimensionless(mmr), Dimensionless(delta), Dimensionless(sigma),
            Hours(tau_small), eps
        ).unwrap();
        let l2 = bernstein_leverage_bound(
            Dimensionless(mmr), Dimensionless(delta), Dimensionless(sigma),
            Hours(tau_big), eps
        ).unwrap();
        prop_assert!(
            l2 <= l1,
            "L(τ={}) = {} should be ≥ L(τ={}) = {}",
            tau_small, l1, tau_big, l2
        );
    }

    /// L^R(τ) ≥ 1 always (at minimum we can hold 1× leverage).
    #[test]
    fn bernstein_leverage_at_least_one(
        mmr in 0.01f64..0.50,
        delta in 0.0001f64..0.2,
        sigma in 0.0001f64..0.2,
        tau in 1.0f64..8760.0,
        eps in 0.001f64..0.10,
    ) {
        let l = bernstein_leverage_bound(
            Dimensionless(mmr), Dimensionless(delta), Dimensionless(sigma),
            Hours(tau), eps
        ).unwrap();
        prop_assert!(l >= 1, "L^R = {} must be ≥ 1", l);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// break_even_hold sanity
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// τ^BE > 0 for all positive μ and positive cost.
    #[test]
    fn break_even_positive_when_mu_positive(
        mu in 0.001f64..5.0,
        cost in 0.0f64..0.05,
        rho in 0.0f64..5.0,
    ) {
        let tau = break_even_hold_at_mean(
            AnnualizedRate(mu), Dimensionless(cost), Dimensionless(rho)
        ).unwrap();
        prop_assert!(tau.0 >= 0.0, "τ^BE = {} must be ≥ 0", tau.0);
        prop_assert!(tau.0.is_finite(), "τ^BE = {} must be finite", tau.0);
    }

    /// τ^BE is monotone non-decreasing in cost.
    /// More cost to break even → must hold longer at the same mu.
    #[test]
    fn break_even_monotone_in_cost(
        mu in 0.01f64..1.0,
        c_small in 0.0f64..0.01,
        c_delta in 0.0001f64..0.04,
    ) {
        let c_big = c_small + c_delta;
        let t1 = break_even_hold_at_mean(
            AnnualizedRate(mu), Dimensionless(c_small), Dimensionless(0.0)
        ).unwrap();
        let t2 = break_even_hold_at_mean(
            AnnualizedRate(mu), Dimensionless(c_big), Dimensionless(0.0)
        ).unwrap();
        prop_assert!(
            t2.0 >= t1.0 - 1e-12,
            "τ^BE(c={}) = {} should be ≤ τ^BE(c={}) = {}",
            c_small, t1.0, c_big, t2.0
        );
    }

    /// τ^BE is monotone non-increasing in μ.
    /// Higher edge → shorter breakeven.
    #[test]
    fn break_even_monotone_in_mu(
        mu_small in 0.001f64..0.1,
        mu_delta in 0.001f64..0.9,
        cost in 0.0001f64..0.02,
    ) {
        let mu_big = mu_small + mu_delta;
        let t1 = break_even_hold_at_mean(
            AnnualizedRate(mu_small), Dimensionless(cost), Dimensionless(0.0)
        ).unwrap();
        let t2 = break_even_hold_at_mean(
            AnnualizedRate(mu_big), Dimensionless(cost), Dimensionless(0.0)
        ).unwrap();
        prop_assert!(
            t2.0 <= t1.0 + 1e-9,
            "τ^BE(μ={}) = {} should be ≥ τ^BE(μ={}) = {}",
            mu_small, t1.0, mu_big, t2.0
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HHI (venue concentration) invariants — Hirschman 1964
// ─────────────────────────────────────────────────────────────────────────────

/// HHI on an n-element equal-weight portfolio equals 1/n.
#[test]
fn hhi_of_equal_weighted_is_one_over_n() {
    use bot_runtime_proxy::hhi_equal_n;
    for n in 1..=16 {
        let h = hhi_equal_n(n);
        assert!(
            (h - 1.0 / n as f64).abs() < 1e-12,
            "HHI({} equal) = {} ≠ 1/{} = {}",
            n,
            h,
            n,
            1.0 / n as f64
        );
    }
}

// HHI ∈ [1/N, 1] for any non-zero exposure vector of length N.
proptest! {
    #[test]
    fn hhi_bounded_by_1_over_n_and_1(
        weights in prop::collection::vec(0.1f64..100.0, 1..16),
    ) {
        use bot_runtime_proxy::hhi_raw;
        let n = weights.len();
        let h = hhi_raw(&weights);
        prop_assert!(h >= 1.0 / n as f64 - 1e-12, "HHI = {} < 1/N = {}", h, 1.0 / n as f64);
        prop_assert!(h <= 1.0 + 1e-12, "HHI = {} > 1", h);
    }
}

/// Local proxy for bot-runtime's HHI so bot-math proptests stay crate-local.
/// Identical formula to `VenueConcentrationCap::hhi`.
mod bot_runtime_proxy {
    pub fn hhi_raw(weights: &[f64]) -> f64 {
        let total: f64 = weights.iter().map(|v| v.max(0.0)).sum();
        if total <= 0.0 {
            return 0.0;
        }
        weights
            .iter()
            .map(|v| {
                let s = v.max(0.0) / total;
                s * s
            })
            .sum()
    }
    pub fn hhi_equal_n(n: usize) -> f64 {
        hhi_raw(&vec![1.0_f64; n])
    }
}
