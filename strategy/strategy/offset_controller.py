"""
offset_controller.py — Aurora-Ω §7.3 + §26.1 Quote offset controller

    d_t = d_base · (1 + γ · σ_m,t) · (1 + λ · p_tox^0.7)

Clipped to d_max (default 20 bp).

Spec: docs/aurora-omega-spec.md §7.3
Pure stdlib.
"""
from __future__ import annotations

from dataclasses import dataclass


DEFAULT_GAMMA: float = 3.0
DEFAULT_LAMBDA: float = 1.5
DEFAULT_D_MAX_BPS: float = 20.0
DEFAULT_D_BASE_BPS: float = 2.0


@dataclass(frozen=True)
class OffsetInputs:
    d_base_bps: float = DEFAULT_D_BASE_BPS
    sigma_m: float = 0.0           # normalized mid-vol (dimensionless)
    p_tox: float = 0.0             # ∈ [0, 1]
    gamma: float = DEFAULT_GAMMA
    lam: float = DEFAULT_LAMBDA
    d_max_bps: float = DEFAULT_D_MAX_BPS


def compute_offset_bps(inp: OffsetInputs) -> float:
    """Return d_t in bps, clipped to [d_base, d_max]."""
    p = max(0.0, min(1.0, inp.p_tox))
    sig = max(0.0, inp.sigma_m)
    raw = inp.d_base_bps * (1.0 + inp.gamma * sig) * (1.0 + inp.lam * (p ** 0.7))
    return min(max(raw, inp.d_base_bps), inp.d_max_bps)
