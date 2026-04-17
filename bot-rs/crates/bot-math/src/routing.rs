//! D.9 — Mandate cap routing.
//!
//! v4 Part 6: conservation law — ∑ slices == vault_gross guaranteed.
//!
//! Python reference: `cost_model.lifecycle_annualized_return` (cap routing section)
//! and `cost_model.Mandate`.

use bot_types::{AnnualizedRate, Mandate, MandateAllocation};

/// Distribute vault gross APY into customer / buffer / reserve slices,
/// respecting per-slice caps and routing excess upward.
///
/// # Algorithm
/// 1. Compute raw slices: customer_raw = R × c_customer, etc.
/// 2. Cap customer at `customer_apy_max`; excess_1 flows to buffer.
/// 3. Cap buffer_raw + excess_1 at `buffer_apy_max`; excess_2 flows to reserve.
/// 4. Reserve = reserve_raw + excess_2 (uncapped).
///
/// Conservation invariant (enforced by `debug_assert`):
///   customer + buffer + reserve = vault_gross  (within 1e-12).
pub fn cap_routing(vault_gross: AnnualizedRate, mandate: &Mandate) -> MandateAllocation {
    let cust_raw = vault_gross.0 * mandate.cut_customer.0;
    let buf_raw = vault_gross.0 * mandate.cut_buffer.0;
    let res_raw = vault_gross.0 * mandate.cut_reserve.0;

    let cust = cust_raw.min(mandate.customer_apy_max.0).max(0.0);
    let excess_1 = (cust_raw - mandate.customer_apy_max.0).max(0.0);

    let buf_with_excess = buf_raw + excess_1;
    let buf = buf_with_excess.min(mandate.buffer_apy_max.0).max(0.0);
    let excess_2 = (buf_with_excess - mandate.buffer_apy_max.0).max(0.0);

    let res = res_raw + excess_2;

    // Conservation invariant (spec §I.1)
    debug_assert!(
        ((cust + buf + res) - vault_gross.0).abs() < 1e-12,
        "cap_routing conservation violated: cust={cust} buf={buf} res={res} sum={} gross={}",
        cust + buf + res,
        vault_gross.0
    );

    MandateAllocation {
        customer: AnnualizedRate(cust),
        buffer: AnnualizedRate(buf),
        reserve: AnnualizedRate(res),
    }
}

/// Mandate floor for vault gross APY R.
///
/// # Formula
/// R_floor = max(customer_min / c_customer, buffer_min / c_buffer)
///
/// This is the minimum vault_gross that satisfies both mandate sub-floors.
pub fn mandate_floor(mandate: &Mandate) -> AnnualizedRate {
    assert!(
        mandate.cut_customer.0 > 0.0,
        "cut_customer must be positive (got {})",
        mandate.cut_customer.0
    );
    assert!(
        mandate.cut_buffer.0 > 0.0,
        "cut_buffer must be positive (got {})",
        mandate.cut_buffer.0
    );
    let cust_floor = mandate.customer_apy_min.0 / mandate.cut_customer.0;
    let buf_floor = mandate.buffer_apy_min.0 / mandate.cut_buffer.0;
    AnnualizedRate(cust_floor.max(buf_floor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_types::Mandate;

    fn default_mandate() -> Mandate {
        Mandate::default()
    }

    /// Conservation: customer + buffer + reserve = vault_gross.
    #[test]
    fn cap_routing_conservation() {
        let m = default_mandate();
        for &gross in &[0.0, 0.05, 0.10, 0.15, 0.20, 0.30, 0.50] {
            let alloc = cap_routing(AnnualizedRate(gross), &m);
            let sum = alloc.customer.0 + alloc.buffer.0 + alloc.reserve.0;
            assert!(
                (sum - gross).abs() < 1e-12,
                "conservation violated at gross={gross}: sum={sum}"
            );
        }
    }

    /// Customer never exceeds its cap.
    #[test]
    fn cap_routing_customer_cap() {
        let m = default_mandate();
        for &gross in &[0.10, 0.20, 0.50] {
            let alloc = cap_routing(AnnualizedRate(gross), &m);
            assert!(
                alloc.customer.0 <= m.customer_apy_max.0 + 1e-12,
                "customer cap violated: {} > {}",
                alloc.customer.0,
                m.customer_apy_max.0
            );
        }
    }

    /// Buffer never exceeds its cap.
    #[test]
    fn cap_routing_buffer_cap() {
        let m = default_mandate();
        for &gross in &[0.10, 0.20, 0.50] {
            let alloc = cap_routing(AnnualizedRate(gross), &m);
            assert!(
                alloc.buffer.0 <= m.buffer_apy_max.0 + 1e-12,
                "buffer cap violated: {} > {}",
                alloc.buffer.0,
                m.buffer_apy_max.0
            );
        }
    }

    /// At vault_gross = 0, all slices are 0.
    #[test]
    fn cap_routing_zero_gross() {
        let m = default_mandate();
        let alloc = cap_routing(AnnualizedRate(0.0), &m);
        assert_eq!(alloc.customer.0, 0.0);
        assert_eq!(alloc.buffer.0, 0.0);
        assert_eq!(alloc.reserve.0, 0.0);
    }

    /// At a moderate vault_gross (e.g. 10%), verify exact values manually.
    /// gross = 0.10, cuts = 0.65/0.25/0.10
    /// cust_raw = 0.065, cap = 0.08 → cust = 0.065, excess = 0
    /// buf_raw  = 0.025, cap = 0.05 → buf  = 0.025, excess = 0
    /// res      = 0.010
    #[test]
    fn cap_routing_no_excess_at_ten_pct() {
        let m = default_mandate();
        let alloc = cap_routing(AnnualizedRate(0.10), &m);
        assert!((alloc.customer.0 - 0.065).abs() < 1e-15);
        assert!((alloc.buffer.0 - 0.025).abs() < 1e-15);
        assert!((alloc.reserve.0 - 0.010).abs() < 1e-15);
    }

    /// At a high vault_gross, excess cascades to reserve.
    /// gross = 0.20
    /// cust_raw = 0.13 → cap 0.08, excess_1 = 0.05
    /// buf_raw  = 0.05, buf+excess = 0.10 → cap 0.05, excess_2 = 0.05
    /// res      = 0.02 + 0.05 = 0.07
    #[test]
    fn cap_routing_excess_cascades() {
        let m = default_mandate();
        let alloc = cap_routing(AnnualizedRate(0.20), &m);
        assert!((alloc.customer.0 - 0.08).abs() < 1e-15);
        assert!((alloc.buffer.0 - 0.05).abs() < 1e-15);
        assert!((alloc.reserve.0 - 0.07).abs() < 1e-15);
    }

    /// mandate_floor with default Mandate.
    /// cust_floor = 0.05 / 0.65 ≈ 0.076923
    /// buf_floor  = 0.02 / 0.25 = 0.08
    /// floor = max(0.076923, 0.08) = 0.08
    #[test]
    fn mandate_floor_default() {
        let m = default_mandate();
        let floor = mandate_floor(&m);
        let expected = (0.05_f64 / 0.65).max(0.02 / 0.25);
        assert!((floor.0 - expected).abs() < 1e-15);
    }
}
