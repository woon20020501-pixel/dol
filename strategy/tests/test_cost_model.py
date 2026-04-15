"""Tests for strategy.cost_model — mandate, slippage, lifecycle, safety gates."""
import math

import pytest

from strategy.cost_model import (
    Mandate,
    LiveInputs,
    target_vault_apy,
    target_vault_apy_floor,
    target_vault_apy_ceil,
    slippage,
    SLIPPAGE_FLOOR,
    SLIPPAGE_CEILING,
    lifecycle_annualized_return,
    round_trip_cost_pct,
    persistence_threshold,
    required_ratio,
    counter_venue_cap,
    oi_cap,
    position_aum_cap,
)


def test_target_vault_apy_within_floor_and_ceil():
    m = Mandate()
    floor = target_vault_apy_floor(m)
    ceil = target_vault_apy_ceil(m)
    target = target_vault_apy(m)
    assert floor <= target <= ceil


def test_target_vault_apy_floor_pm_targets():
    m = Mandate()
    # max(0.05/0.65, 0.02/0.25) = max(0.0769.., 0.08) = 0.08
    assert target_vault_apy_floor(m) == pytest.approx(0.08)


def test_slippage_zero_notional_returns_zero():
    assert slippage(0.0, 1_000_000, 1_000_000) == 0.0


def test_slippage_floor_and_ceiling_enforced():
    # Tiny notional with huge depth — should hit floor
    assert slippage(1.0, 10**12, 10**12) == SLIPPAGE_FLOOR
    # Huge notional with tiny depth — should hit ceiling
    assert slippage(10**12, 1.0, 1.0) == SLIPPAGE_CEILING


def test_slippage_monotone_increasing_in_notional():
    depths_oi, depths_vol = 5_000_000, 5_000_000
    small = slippage(1_000, depths_oi, depths_vol)
    medium = slippage(100_000, depths_oi, depths_vol)
    big = slippage(1_000_000, depths_oi, depths_vol)
    assert small <= medium <= big


def test_persistence_threshold_increasing_with_z_and_decreasing_with_T():
    m = Mandate()
    p_short = persistence_threshold(100, m)
    p_long = persistence_threshold(1000, m)
    # Longer sample → we can demand less over-50% to reject fair-coin null
    assert p_long < p_short
    assert 0.5 < p_long < 1.0


def test_persistence_threshold_zero_T_returns_one():
    assert persistence_threshold(0, Mandate()) == 1.0


def test_required_ratio_inf_when_snr_low():
    m = Mandate()
    # Z_ratio_downside default 1.65; snr below 1.70 → inf
    assert required_ratio(1.5, m) == float("inf")


def test_required_ratio_positive_finite_for_high_snr():
    m = Mandate()
    r = required_ratio(10.0, m)
    assert 1.0 < r < 2.0


def test_counter_venue_cap_headroom():
    assert counter_venue_cap(0) == 0.0
    # With 3 counters, cap = 1.20 / 3 = 0.40
    assert counter_venue_cap(3) == pytest.approx(0.40)
    # Clamped at 1.0 when 1 counter → 1.20 clamps
    assert counter_venue_cap(1) == 1.0


def test_position_aum_cap_zero_active_returns_zero():
    m = Mandate()
    assert position_aum_cap(idle_frac=0.5, leverage=2, n_active=0, m=m) == 0.0


def test_position_aum_cap_formula():
    m = Mandate()
    # m_pos = (1-α) * L / (2 * N_target)
    val = position_aum_cap(idle_frac=0.5, leverage=2, n_active=4, m=m)
    assert val == pytest.approx((0.5) * 2 / (2 * 4))


def test_oi_cap_zero_oi_returns_zero():
    inputs = LiveInputs(
        timestamp_ms=0, aum_usd=1000, r_idle=0.04,
        funding_rate_h={}, open_interest_usd={},
        volume_24h_usd={}, fee_maker={}, fee_taker={}, bridge_fee_round_trip={},
    )
    assert oi_cap("FOO", "pacifica", inputs) == 0.0


def test_oi_cap_bounds_turnover_scale():
    # High turnover → scale clamped at 1.4; cap = 0.05 * 1.4 = 0.07
    inputs = LiveInputs(
        timestamp_ms=0, aum_usd=1000, r_idle=0.04,
        funding_rate_h={},
        open_interest_usd={("FOO", "pacifica"): 1_000_000},
        volume_24h_usd={("FOO", "pacifica"): 50_000_000},  # 50x turnover
        fee_maker={}, fee_taker={}, bridge_fee_round_trip={},
    )
    assert oi_cap("FOO", "pacifica", inputs) == pytest.approx(0.07)


def test_lifecycle_annualized_return_breakeven_math():
    result = lifecycle_annualized_return(
        per_pair_spread_apy=0.10,
        commitment_hold_h=168.0,
        c_round_trip=0.0015,
        leverage=2,
        alpha=0.5,
        r_idle=0.04,
    )
    rot = 8760.0 / 168.0
    # gross_on_margin = L/2 * s = 1 * 0.10 = 0.10
    assert result["gross_on_margin"] == pytest.approx(0.10)
    assert result["rotations_per_year"] == pytest.approx(rot)
    # Annual cost on margin = L/2 * c * rot
    assert result["annual_cost_on_margin"] == pytest.approx(0.0015 * rot)
    # Idle contribution = alpha * r_idle = 0.02
    assert result["idle_contribution"] == pytest.approx(0.02)


def test_lifecycle_cap_routing_customer_capped_at_8pct():
    # Very high spread — should push customer to cap 0.08
    result = lifecycle_annualized_return(
        per_pair_spread_apy=1.0,
        commitment_hold_h=168.0,
        c_round_trip=0.0,
        leverage=4,
        alpha=0.5,
        r_idle=0.04,
    )
    assert result["customer"] == pytest.approx(0.08)
    assert result["buffer"] <= 0.05 + 1e-9
    # Reserve absorbs the overflow
    assert result["reserve"] > 0.10 * result["vault_gross"]


def test_round_trip_cost_pct_sums_components():
    inputs = LiveInputs(
        timestamp_ms=0, aum_usd=1000, r_idle=0.04,
        funding_rate_h={},
        open_interest_usd={("X", "pacifica"): 1e9, ("X", "backpack"): 1e9},
        volume_24h_usd={("X", "pacifica"): 1e9, ("X", "backpack"): 1e9},
        fee_maker={"pacifica": 0.0001, "backpack": 0.0002},
        fee_taker={},
        bridge_fee_round_trip={("pacifica", "backpack"): 0.0005},
    )
    c = round_trip_cost_pct("X", "pacifica", "backpack", 1_000.0, inputs)
    # fees: pacifica 0.0002 (open+close), backpack 0.0004, bridge 0.0005, slippage 2 * SLIPPAGE_FLOOR per venue
    expected_fees = 0.0002 + 0.0004 + 0.0005
    assert c >= expected_fees
    # Must be positive
    assert c > 0
