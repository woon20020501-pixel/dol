"""Miscellaneous unit tests covering smaller utility modules."""
import pytest

from strategy.cost_model import Mandate
from strategy.cost_model import target_vault_apy_floor, target_vault_apy_ceil


def test_mandate_defaults_are_frozen():
    m = Mandate()
    with pytest.raises((AttributeError, Exception)):
        m.customer_apy_min = 0.0  # dataclass frozen


def test_mandate_cuts_sum_to_one():
    m = Mandate()
    assert m.cut_customer + m.cut_buffer + m.cut_reserve == pytest.approx(1.0)


def test_mandate_aum_caps_valid_range():
    m = Mandate()
    assert 0.0 < m.aum_buffer_floor < m.aum_idle_cap < 1.0 + 1e-9


def test_mandate_dex_whitelist_has_pacifica():
    m = Mandate()
    assert "pacifica" in m.dex_venues


def test_target_vault_apy_floor_less_than_ceil():
    m = Mandate()
    assert target_vault_apy_floor(m) < target_vault_apy_ceil(m)


# Depth threshold — small module with apply_depth_threshold helper
def test_depth_threshold_module_imports():
    from strategy import depth_threshold  # noqa: F401


def test_fractal_delta_module_imports():
    from strategy import fractal_delta  # noqa: F401


def test_fair_value_oracle_module_imports():
    from strategy import fair_value_oracle  # noqa: F401
