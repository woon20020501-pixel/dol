"""Tests for strategy.funding_cycle_lock — Aurora-Ω §3.1 cycle lock."""
import pytest

from strategy.funding_cycle_lock import (
    CycleState,
    DEFAULT_CYCLE_SECONDS,
    cycle_index,
    cycle_phase,
    seconds_to_cycle_end,
    is_locked,
    open_cycle,
    enforce,
    would_violate_lock,
)


def test_cycle_index_at_boundary_and_inside():
    assert cycle_index(0.0) == 0
    assert cycle_index(3599.9) == 0
    assert cycle_index(3600.0) == 1
    assert cycle_index(7200.0) == 2


def test_cycle_index_requires_positive_period():
    with pytest.raises(ValueError):
        cycle_index(100.0, cycle_seconds=0)


def test_cycle_phase_range():
    assert cycle_phase(0.0) == 0.0
    assert cycle_phase(1800.0) == pytest.approx(0.5)
    assert cycle_phase(3599.0) == pytest.approx(3599 / 3600)


def test_seconds_to_cycle_end_at_start_is_full_period():
    assert seconds_to_cycle_end(0.0) == DEFAULT_CYCLE_SECONDS
    assert seconds_to_cycle_end(3599.0) == pytest.approx(1.0)


def test_is_locked_none_state_returns_false():
    assert is_locked(None, now=123.0) is False


def test_is_locked_same_cycle_true_next_cycle_false():
    state = open_cycle(now=100.0, h_c=1, N_c=1000.0)
    assert is_locked(state, now=200.0) is True
    # Cross the cycle boundary — 3600 seconds later we're in a new cycle
    assert is_locked(state, now=100.0 + DEFAULT_CYCLE_SECONDS) is False


def test_open_cycle_invalid_direction_raises():
    with pytest.raises(ValueError):
        open_cycle(now=0.0, h_c=2, N_c=100.0)


def test_open_cycle_negative_notional_raises():
    with pytest.raises(ValueError):
        open_cycle(now=0.0, h_c=1, N_c=-10.0)


def test_enforce_no_state_passes_through():
    h, N = enforce(None, now=0.0, proposed_h=1, proposed_N=500.0)
    assert (h, N) == (1, 500.0)


def test_enforce_locked_blocks_direction_flip():
    state = open_cycle(now=0.0, h_c=1, N_c=1000.0)
    h, N = enforce(state, now=100.0, proposed_h=-1, proposed_N=1000.0)
    assert h == 1
    assert N == 1000.0


def test_enforce_locked_blocks_notional_change():
    state = open_cycle(now=0.0, h_c=1, N_c=1000.0)
    h, N = enforce(state, now=100.0, proposed_h=1, proposed_N=500.0)
    # Notional is frozen to N_c
    assert N == 1000.0


def test_enforce_emergency_override_allows_flip():
    state = open_cycle(now=0.0, h_c=1, N_c=1000.0)
    h, N = enforce(state, now=100.0, proposed_h=-1, proposed_N=250.0,
                   emergency_override=True)
    assert (h, N) == (-1, 250.0)


def test_enforce_after_cycle_rollover_passes_through():
    state = open_cycle(now=0.0, h_c=1, N_c=1000.0)
    h, N = enforce(state, now=DEFAULT_CYCLE_SECONDS + 1, proposed_h=-1, proposed_N=500.0)
    assert (h, N) == (-1, 500.0)


def test_enforce_invalid_proposed_direction_raises():
    with pytest.raises(ValueError):
        enforce(None, now=0.0, proposed_h=5, proposed_N=100.0)


def test_enforce_invalid_proposed_notional_raises():
    with pytest.raises(ValueError):
        enforce(None, now=0.0, proposed_h=1, proposed_N=-1.0)


def test_would_violate_lock_detects_flip():
    state = open_cycle(now=0.0, h_c=1, N_c=1000.0)
    assert would_violate_lock(state, now=100.0, proposed_h=-1) is True
    assert would_violate_lock(state, now=100.0, proposed_h=1) is False
    # Going flat from a locked direction is also a violation
    assert would_violate_lock(state, now=100.0, proposed_h=0) is True


def test_would_violate_lock_no_state_is_false():
    assert would_violate_lock(None, now=100.0, proposed_h=-1) is False
