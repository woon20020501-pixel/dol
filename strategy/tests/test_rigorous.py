"""Tests for strategy.rigorous — regime filter, leverage, exit decisions."""
import math
import random

import pytest

from strategy.cost_model import LiveInputs, Mandate
from strategy.rigorous import (
    RigorousCandidate,
    RigorousState,
    build_spread_series,
    filter_candidate_rigorous,
    compute_rigorous_state,
    required_leverage_rigorous,
    evaluate_exit_rigorous,
    ExitDecision,
)
from strategy.stochastic import _generate_ou_sample


def _empty_inputs(r_idle: float = 0.04) -> LiveInputs:
    return LiveInputs(
        timestamp_ms=0, aum_usd=1_000_000, r_idle=r_idle,
        funding_rate_h={}, open_interest_usd={}, volume_24h_usd={},
        fee_maker={"pacifica": 0.00015, "backpack": 0.00020,
                   "hyperliquid": 0.00020, "lighter": 0.00020},
        fee_taker={}, bridge_fee_round_trip={},
        funding_history_h={}, basis_divergence_history={},
        vault_daily_returns=[],
    )


def test_required_leverage_rigorous_defaults_to_one():
    # zero signal → just clamped to min_leverage floor (default 1)
    L = required_leverage_rigorous(0.0, r_idle=0.04, target_apy=0.077)
    assert L == 1


def test_required_leverage_rigorous_bounded_by_ten():
    # Massive target, tiny spread → needs huge leverage → capped at 10
    L = required_leverage_rigorous(0.001, r_idle=0.04, target_apy=1.0)
    assert L == 10


def test_required_leverage_rigorous_monotone_in_target():
    spread = 0.08
    L_low = required_leverage_rigorous(spread, r_idle=0.04, target_apy=0.05)
    L_high = required_leverage_rigorous(spread, r_idle=0.04, target_apy=0.15)
    assert L_high >= L_low


def test_build_spread_series_empty_when_no_history():
    inputs = _empty_inputs()
    ts, spread = build_spread_series("BTC", "backpack", inputs)
    assert ts == [] and spread == []


def test_build_spread_series_aligns_timestamps():
    inputs = _empty_inputs()
    inputs.funding_history_h[("BTC", "pacifica")] = [(1, 0.01), (2, 0.02), (3, 0.03)]
    inputs.funding_history_h[("BTC", "backpack")] = [(2, 0.05), (3, 0.07), (4, 0.10)]
    ts, spread = build_spread_series("BTC", "backpack", inputs)
    # Only timestamps 2 and 3 are common → spread = backpack - pacifica
    assert ts == [2, 3]
    assert spread == [pytest.approx(0.05 - 0.02), pytest.approx(0.07 - 0.03)]


def test_filter_candidate_rejects_pacifica_counter():
    m = Mandate()
    inputs = _empty_inputs()
    assert filter_candidate_rigorous("BTC", "pacifica", inputs, m) is None


def test_filter_candidate_rejects_non_whitelisted_counter():
    m = Mandate()
    inputs = _empty_inputs()
    assert filter_candidate_rigorous("BTC", "binance", inputs, m) is None


def test_filter_candidate_rejects_insufficient_history():
    m = Mandate()
    inputs = _empty_inputs()
    # Only 5 points of history — way below persistence_lookback_h_min (168)
    inputs.funding_history_h[("BTC", "pacifica")] = [(i, 0.001) for i in range(5)]
    inputs.funding_history_h[("BTC", "backpack")] = [(i, 0.003) for i in range(5)]
    inputs.funding_rate_h[("BTC", "pacifica")] = 0.001
    inputs.funding_rate_h[("BTC", "backpack")] = 0.003
    assert filter_candidate_rigorous("BTC", "backpack", inputs, m) is None


def test_compute_rigorous_state_empty_universe_returns_all_idle_plan():
    m = Mandate()
    inputs = _empty_inputs()
    state = compute_rigorous_state(inputs, m)
    assert isinstance(state, RigorousState)
    assert state.n_passing_filters == 0
    assert state.candidates == []
    assert state.leverage == 1
    assert state.chance_constrained.feasible is False
    # Fallback: idle accrues r_idle × idle_cap
    assert state.chance_constrained.vault_5pct_apy == pytest.approx(
        inputs.r_idle * m.aum_idle_cap
    )


def test_compute_rigorous_state_target_and_floor_match_mandate_helpers():
    m = Mandate()
    inputs = _empty_inputs()
    state = compute_rigorous_state(inputs, m)
    # target_vault_apy and target_vault_apy_floor should be attached
    assert state.target_floor_apy == pytest.approx(0.08)
    assert state.target_vault_apy > state.target_floor_apy - 1e-9
