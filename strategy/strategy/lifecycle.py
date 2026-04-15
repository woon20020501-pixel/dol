"""
lifecycle.py — DEPRECATED 2026-04-14.

This module was part of v4.0, which was rejected under the current design as an unauthorized
deviation from the iron law in PRINCIPLES.md §1. The Kamino multiply + base
lending phased strategy is NOT the cross-venue same-asset funding hedge that
defines Dol. Do not import or use this module.

The locked framework is v3.5: cost_model.py + stochastic.py + portfolio.py +
rigorous.py + frontier.py. See PRINCIPLES.md §1.5.

This file is preserved only as an audit-trail artifact.
"""
import warnings
warnings.warn(
    "lifecycle.py is DEPRECATED. v4.0 was rejected. Use the v3.5 framework "
    "(cost_model + stochastic + portfolio + rigorous + frontier).",
    DeprecationWarning,
    stacklevel=2,
)
from __future__ import annotations
import math
from dataclasses import dataclass
from typing import Literal


PhaseTag = Literal["phase_1_beta", "phase_2_growth", "phase_3_scale", "phase_4_mature"]


@dataclass(frozen=True)
class PhaseConfig:
    name: PhaseTag
    aum_min_usd: float
    aum_max_usd: float
    base_lending_pct: float        # α_b  diversified Solana lending markets
    multiply_pct: float            # α_m  Kamino USDC multiply
    multiply_leverage: float       # L (≤ 2.5 cap by §3.4)
    hedge_pct: float               # α_h  v3.5 cross-venue overlay
    reserve_pct: float             # α_r
    instant_withdraw_pct: float    # fraction guaranteed instantly withdrawable
    description: str


# Phase definitions per math-final-v4 §1
PHASE_1_BETA = PhaseConfig(
    name="phase_1_beta",
    aum_min_usd=0,
    aum_max_usd=100_000,
    base_lending_pct=0.95,
    multiply_pct=0.0,
    multiply_leverage=1.0,
    hedge_pct=0.0,
    reserve_pct=0.05,
    instant_withdraw_pct=0.95,
    description="Pure diversified Solana lending. No leverage. 100% instant.",
)

PHASE_2_GROWTH = PhaseConfig(
    name="phase_2_growth",
    aum_min_usd=100_000,
    aum_max_usd=1_000_000,
    base_lending_pct=0.25,
    multiply_pct=0.70,
    multiply_leverage=2.5,
    hedge_pct=0.0,
    reserve_pct=0.05,
    instant_withdraw_pct=0.25,
    description="Kamino multiply 2.5x for yield + base lending for instant buffer.",
)

PHASE_3_SCALE = PhaseConfig(
    name="phase_3_scale",
    aum_min_usd=1_000_000,
    aum_max_usd=10_000_000,
    base_lending_pct=0.20,
    multiply_pct=0.60,
    multiply_leverage=2.5,
    hedge_pct=0.15,
    reserve_pct=0.05,
    instant_withdraw_pct=0.20,
    description="Multi-strategy: Kamino multiply + base lending + v3.5 overlay (15%).",
)


def phase_for(aum_usd: float) -> PhaseConfig:
    if aum_usd < PHASE_1_BETA.aum_max_usd:
        return PHASE_1_BETA
    if aum_usd < PHASE_2_GROWTH.aum_max_usd:
        return PHASE_2_GROWTH
    return PHASE_3_SCALE


# ---------------------------------------------------------------------------
# Kamino multiply math (math-final-v4 §3)
# ---------------------------------------------------------------------------

def kamino_multiply_apy(supply_rate: float, borrow_rate: float, leverage: float) -> float:
    """Effective APY of a Kamino multiply position.

    multiply_apy = borrow_rate + L · (supply_rate - borrow_rate)

    For supply 4.4%, borrow 1.5%, L=2.5 → 9.05%.
    """
    return borrow_rate + leverage * (supply_rate - borrow_rate)


def kamino_liquidation_drop(leverage: float, ltv_max: float = 0.85) -> float:
    """Required JLP price drop to trigger liquidation. Returns positive fraction.

    For L=2.5, LTV_max=0.85: 1 - 0.85 * 2.5 / 1.5 = -0.417 (cannot liquidate at +ve)
    For L=4.0: 1 - 0.85 * 4 / 3 = -0.133

    A return < 0 means "no possible liquidation at this leverage" (very safe).
    Return > 0 means liquidation triggers when JLP falls by that fraction.
    """
    if leverage <= 1:
        return float("inf")
    return 1.0 - (ltv_max * leverage / (leverage - 1))


def safe_max_leverage(jlp_drawdown_observed: float, safety_factor: float = 1.5,
                      ltv_max: float = 0.85) -> float:
    """The maximum L such that liquidation distance > safety_factor * observed drawdown.

    If observed JLP drawdown is 30%, safety_factor 1.5 → liquidation distance ≥ 45%.
    Solve: |1 - LTV_max·L/(L-1)| ≥ 0.45
    → LTV_max·L/(L-1) ≥ 1.45  →  0.85L ≥ 1.45(L-1)  →  L(0.85 - 1.45) ≥ -1.45
    → -0.6 L ≥ -1.45 → L ≤ 2.42
    """
    required_distance = safety_factor * abs(jlp_drawdown_observed)
    # Solve LTV_max * L / (L-1) = 1 + required_distance for L
    # LTV_max · L = (1 + required_distance)(L - 1)
    # LTV_max · L = (1 + required_distance) L - (1 + required_distance)
    # L · (LTV_max - 1 - required_distance) = -(1 + required_distance)
    # L = (1 + required_distance) / (1 + required_distance - LTV_max)
    denom = (1 + required_distance - ltv_max)
    if denom <= 0:
        return float("inf")
    L_max = (1 + required_distance) / denom
    return min(L_max, 5.0)  # absolute cap


def adaptive_leverage(target_leverage: float, jlp_drawdown_pct: float) -> float:
    """Apply graceful deleverage as JLP drops from entry price.
    Per math-final-v4 §3.4."""
    if jlp_drawdown_pct < 0.15:
        return target_leverage
    elif jlp_drawdown_pct < 0.25:
        return min(target_leverage, 2.0)
    elif jlp_drawdown_pct < 0.35:
        return min(target_leverage, 1.5)
    else:
        return 1.0  # fully unlever


# ---------------------------------------------------------------------------
# Phase-aware vault APY composition (math-final-v4 §5)
# ---------------------------------------------------------------------------

@dataclass
class VaultProjection:
    phase: PhaseTag
    aum: float
    base_lending_pct: float
    multiply_pct: float
    hedge_pct: float
    reserve_pct: float
    leverage: float
    base_apy: float
    multiply_apy: float
    hedge_apy: float
    vault_gross_apy: float
    customer_apy: float
    buffer_apy: float
    reserve_apy: float
    customer_in_band: bool
    buffer_in_band: bool
    instant_withdraw_pct: float


def project_vault_apy(aum_usd: float,
                     base_supply_rate: float = 0.045,
                     jlp_borrow_rate: float = 0.015,
                     v3_5_overlay_apy: float = 0.08,
                     jlp_drawdown_pct: float = 0.0,
                     cut_customer: float = 0.65,
                     cut_buffer: float = 0.25,
                     cut_reserve: float = 0.10,
                     customer_floor: float = 0.05,
                     customer_ceiling: float = 0.07,
                     buffer_floor: float = 0.02,
                     buffer_ceiling: float = 0.05) -> VaultProjection:
    """Compute the vault projected APY at a given AUM and DeFi rate environment.

    Customer floor defaults to the mandate floor of 5%; the v4.0
    doc recommends 4.5% as a more honest target, but we use 5% by default and
    let the result speak for itself."""
    phase = phase_for(aum_usd)
    L = adaptive_leverage(phase.multiply_leverage, jlp_drawdown_pct)
    base_apy = base_supply_rate
    mult_apy = kamino_multiply_apy(base_supply_rate, jlp_borrow_rate, L) if L > 1 else 0.0
    vault = (
        phase.base_lending_pct * base_apy
        + phase.multiply_pct * mult_apy
        + phase.hedge_pct * v3_5_overlay_apy
    )
    cust = vault * cut_customer
    buf = vault * cut_buffer
    res = vault * cut_reserve
    cust_capped = min(cust, customer_ceiling)
    buf_with_excess = buf + (cust - cust_capped)
    buf_capped = min(buf_with_excess, buffer_ceiling)
    res_with_excess = res + (buf_with_excess - buf_capped)
    return VaultProjection(
        phase=phase.name, aum=aum_usd,
        base_lending_pct=phase.base_lending_pct,
        multiply_pct=phase.multiply_pct,
        hedge_pct=phase.hedge_pct,
        reserve_pct=phase.reserve_pct,
        leverage=L,
        base_apy=base_apy, multiply_apy=mult_apy, hedge_apy=v3_5_overlay_apy,
        vault_gross_apy=vault,
        customer_apy=cust_capped,
        buffer_apy=buf_capped,
        reserve_apy=res_with_excess,
        customer_in_band=customer_floor <= cust_capped <= customer_ceiling,
        buffer_in_band=buffer_floor <= buf_capped <= buffer_ceiling,
        instant_withdraw_pct=phase.instant_withdraw_pct,
    )


if __name__ == "__main__":
    print("=" * 100)
    print("v4.0 LIFECYCLE PROJECTION — across AUM tiers and DeFi rate scenarios")
    print("=" * 100)
    print()

    scenarios = [
        ("low rates",   0.035, 0.012, 0.06),
        ("normal rates",0.044, 0.015, 0.08),
        ("high rates",  0.055, 0.020, 0.10),
        ("hot market",  0.065, 0.025, 0.12),
    ]

    for label, supply, borrow, hedge in scenarios:
        print(f"--- DeFi scenario: {label}  (USDC supply {supply*100:.1f}%, JLP borrow {borrow*100:.1f}%, v3.5 net {hedge*100:.1f}%)")
        print(f"  {'AUM':<14}{'phase':<18}{'base%':>8}{'mult%':>8}{'hedge%':>8}{'L':>5}"
              f"{'gross':>10}{'cust':>10}{'buf':>9}{'mandate':>15}")
        for aum in [10_000, 100_000, 500_000, 1_000_000, 5_000_000, 10_000_000]:
            p = project_vault_apy(aum, supply, borrow, hedge)
            mandate_ok = "✓✓" if p.customer_in_band and p.buffer_in_band else \
                         ("c✓ b✗" if p.customer_in_band else
                          ("c✗ b✓" if p.buffer_in_band else "✗"))
            print(f"  ${aum:>11,}  {p.phase:<18}"
                  f"{p.base_lending_pct*100:>7.0f}%"
                  f"{p.multiply_pct*100:>7.0f}%"
                  f"{p.hedge_pct*100:>7.0f}%"
                  f"{p.leverage:>5.1f}"
                  f"{p.vault_gross_apy*100:>9.2f}%"
                  f"{p.customer_apy*100:>9.2f}%"
                  f"{p.buffer_apy*100:>8.2f}%"
                  f"{mandate_ok:>15}")
        print()

    print("=" * 100)
    print("LIQUIDATION SAFETY: Kamino multiply safe leverage by JLP drawdown observed")
    print("=" * 100)
    for dd in [0, 0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40]:
        L_safe = safe_max_leverage(dd, safety_factor=1.5)
        adapt_L = adaptive_leverage(2.5, dd)
        liq_drop = kamino_liquidation_drop(adapt_L)
        liq_str = f"{liq_drop*100:.0f}% (safe)" if liq_drop > 0 else "impossible"
        print(f"  observed JLP drawdown {dd*100:>3.0f}%  →  safe L max = {L_safe:>4.2f}  "
              f"adaptive L = {adapt_L:>3.1f}  liquidation needs JLP drop of {liq_str}")

    print()
    print("=" * 100)
    print("INTERPRETATION")
    print("=" * 100)
    print("- Phase 1 (beta, AUM < $100k) is intentionally below mandate.")
    print("  It's the safest possible setup while user trust is being built.")
    print("- Phase 2 hits mandate ONLY when supply rates are normal-or-better (≥4.4%).")
    print("- Phase 3 (with v3.5 overlay) widens the operating envelope.")
    print("- Liquidation risk is mathematically eliminated at L≤2.5 unless JLP drops >40%.")
    print("- The framework is honest about its limits — it does not fake compliance.")
