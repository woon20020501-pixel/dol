"""
latency_penalty.py — Aurora-Ω §18 execution cost layer (three-term split)

Review response #4 (2026-04-15): the earlier single-term "latency penalty"
conflated three separate effects. The corrected canonical form is:

    C_i(q) = C_i^impact(q) + C_i^timing(q) + C_i^congestion(q)

where:

    (1) Kyle-style impact      C_i^impact = η · D · (q/D)^(1+δ)       [USD]
    (2) Timing-risk            C_i^timing = q · σ_price · √τ          [USD]
    (3) Optional congestion    C_i^cong   = α · τ · σ_flow · q² / D   [USD]

Units:
    q              USD
    D              USD (effective depth at mid)
    τ              seconds
    σ_price        1/√s     (dimensionless return per √s)
    σ_flow         1/s
    α, η, δ        dimensionless

All three terms are in USD. The timing-risk term replaces the old
single-term interpretation as the dominant latency cost for modest
delays; congestion is an OPTIONAL add-on for venues with rapidly
varying book thickness and defaults to α=0.

Spec: docs/aurora-omega-spec.md §18.1–§18.4
Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass


# §18.3 congestion default — off (α_i = 0) until per-venue evidence shows
# the congestion term is needed. Review response: "ship with α_i = 0".
DEFAULT_ALPHA: float = 0.0

# §18.2 timing-risk conservative default σ_price — dimensionless per √s.
# Typical crypto perp near-the-clock volatility is O(1e-3 / √s) at 1s scale;
# 5e-4 is a deliberately conservative choice (undershoots vol → overstates
# latency risk → gates over-fire rather than under-fire). Production deployment
# replaces this with a rolling per-venue estimate.
DEFAULT_SIGMA_PRICE_PER_SQRTS: float = 5e-4


@dataclass(frozen=True)
class VenueCostInputs:
    venue: str
    q_usd: float                                                    # order size, USD
    depth_usd: float                                                # book depth at mid, USD
    tau_s: float                                                    # measured round-trip latency, seconds
    sigma_flow_per_s: float = 0.0                                   # flow volatility, 1/s (for optional congestion term)
    sigma_price_per_sqrts: float = DEFAULT_SIGMA_PRICE_PER_SQRTS    # timing-risk volatility, 1/√s
    eta: float = 0.01                                               # impact scale constant (dimensionless)
    alpha: float = DEFAULT_ALPHA                                    # congestion scale (default 0 = off)


# ---------------------------------------------------------------------------
# Three cost terms
# ---------------------------------------------------------------------------


def impact_cost(inp: VenueCostInputs, delta: float) -> float:
    """§18.1 Kyle-style impact: η · D · (q/D)^(1+δ)."""
    if inp.depth_usd <= 0.0 or inp.q_usd <= 0.0:
        return 0.0
    ratio = inp.q_usd / inp.depth_usd
    try:
        return inp.eta * inp.depth_usd * (ratio ** (1.0 + delta))
    except (OverflowError, ValueError):
        return float("inf")


def timing_risk_cost(
    q_usd: float,
    sigma_price_per_sqrts: float,
    tau_s: float,
) -> float:
    """§18.2 timing-risk cost: q · σ_price · √τ.

    Interpretation: while an order sits for τ seconds, the reference price
    moves by approximately σ_price · √τ in standard deviation. The expected
    signed loss is on the order of q · σ_price · √τ. This is a TIMING
    VARIANCE cost, not a Kyle impact — it reflects the price-movement
    risk incurred by the execution delay itself.

    Returns USD. Non-positive inputs return 0.
    """
    if q_usd <= 0.0 or sigma_price_per_sqrts <= 0.0 or tau_s <= 0.0:
        return 0.0
    return q_usd * sigma_price_per_sqrts * math.sqrt(tau_s)


def congestion_cost(inp: VenueCostInputs) -> float:
    """§18.3 OPTIONAL congestion cost: α · τ · σ_flow · q² / D.

    Off by default (α_i = 0). Enable per-venue only when Phase 1 data
    shows the congestion term is materially above the timing-risk term.
    """
    if inp.depth_usd <= 0.0 or inp.alpha <= 0.0 or inp.sigma_flow_per_s <= 0.0:
        return 0.0
    return inp.alpha * inp.tau_s * inp.sigma_flow_per_s * (inp.q_usd ** 2) / inp.depth_usd


# Backward-compat alias — older callers used `latency_cost` for what was
# the single-term "latency penalty". That term is now the congestion term.
# New callers should use `congestion_cost` directly.
def latency_cost(inp: VenueCostInputs) -> float:
    """Deprecated alias for congestion_cost. Retained for backward compatibility.

    Behavior changed in v1.1: with the default α = 0, this function now
    returns 0 by default. Callers that previously depended on a non-zero
    value must explicitly set `alpha` in VenueCostInputs. The timing-risk
    component is now in `timing_risk_cost`.
    """
    if inp.depth_usd <= 0.0:
        return float("inf")
    return congestion_cost(inp)


# ---------------------------------------------------------------------------
# Total + breakdown
# ---------------------------------------------------------------------------


def total_venue_cost(inp: VenueCostInputs, delta: float) -> float:
    """C_i(q) = impact + timing + congestion. Returns USD."""
    return (
        impact_cost(inp, delta)
        + timing_risk_cost(inp.q_usd, inp.sigma_price_per_sqrts, inp.tau_s)
        + congestion_cost(inp)
    )


@dataclass(frozen=True)
class VenueCostBreakdown:
    venue: str
    impact: float
    timing: float
    congestion: float
    total: float

    # Backward-compat: older callers referenced a `.latency` field.
    @property
    def latency(self) -> float:
        return self.congestion


def breakdown(inp: VenueCostInputs, delta: float) -> VenueCostBreakdown:
    imp = impact_cost(inp, delta)
    tim = timing_risk_cost(inp.q_usd, inp.sigma_price_per_sqrts, inp.tau_s)
    con = congestion_cost(inp)
    return VenueCostBreakdown(
        venue=inp.venue,
        impact=imp,
        timing=tim,
        congestion=con,
        total=imp + tim + con,
    )
