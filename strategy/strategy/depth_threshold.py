"""
depth_threshold.py — Aurora-Ω §5 Depth Threshold Rule

The depth-aware allocation (Aurora-Ω §4.1) distributes hedge notional
across venues in proportion to a weight that mixes volume and depth:

    q_i* ∝ V_i^(1/(1+δ)) · D_i^(-δ/(1+δ))

This weight DOWN-weights shallow venues but never zeroes them out, so a
venue with D_i = $200 of depth can still receive a small slice. In
practice that slice is pure slippage — the shallow venue can't absorb
any meaningful order without walking the book.

§5 fixes this by applying a HARD CUT at D_min (default $5000) before the
allocator sees the venue list, then redistributing the leftover notional
among the survivors by re-running the depth-aware weighting on the
survivor set only. This is the "leftover redistribution" rule.

A fail-safe fallback: if all survivors have zero weight (edge case when
their volumes or depths underflow), route everything to the deepest
surviving book.

Spec: docs/aurora-omega-spec.md §5.1, §5.2
Pure stdlib; no external dependencies.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Iterable


# Default depth floor. This is the Aurora-Ω §5.1 value. Callers may pass a
# larger value (e.g. during high volatility) but must not go below this
# without policy sign-off — a cut below $5000 re-exposes the allocator to
# shallow-venue slippage the rule is designed to prevent.
DEFAULT_D_MIN_USD: float = 5000.0


@dataclass(frozen=True)
class VenueSlot:
    """Immutable venue record for the allocator.

    Attributes
    ----------
    venue : str
        Venue identifier.
    volume_usd : float
        Effective 24h volume (or other window chosen by the caller), USD.
    depth_usd : float
        Effective book depth at the reference price, USD. This is the
        quantity compared against D_min for the hard cut.
    allocated : float
        Notional allocated to this venue after depth_threshold runs.
        Defaults to 0.0 for fresh inputs.
    """

    venue: str
    volume_usd: float
    depth_usd: float
    allocated: float = 0.0


def _depth_aware_weight(v: VenueSlot, delta: float) -> float:
    """V_i^(1/(1+δ)) · D_i^(-δ/(1+δ)).

    Returns 0.0 if either V_i or D_i is non-positive, so dead venues fall
    out of the normalization naturally. We clamp delta to (-1+eps, 10] to
    avoid blowups when fractal_delta produces extreme values during
    thin-universe tests.
    """
    if v.depth_usd <= 0.0 or v.volume_usd <= 0.0:
        return 0.0
    d = max(min(delta, 10.0), -0.999)  # guard against numerical edge cases
    exp_v = 1.0 / (1.0 + d)
    exp_d = -d / (1.0 + d)
    try:
        return (v.volume_usd ** exp_v) * (v.depth_usd ** exp_d)
    except (OverflowError, ValueError):
        return 0.0


def apply_depth_threshold(
    slots: Iterable[VenueSlot],
    total_notional: float,
    delta: float,
    d_min: float = DEFAULT_D_MIN_USD,
) -> list[VenueSlot]:
    """Apply the Aurora-Ω §5 depth threshold + leftover redistribution.

    Steps
    -----
    1. Partition slots into survivors (D_i >= d_min) and cut (D_i < d_min).
    2. If the survivor set is empty, return all slots with allocated=0.0
       (the caller is expected to halt trading — the whole universe is
       too shallow to execute against).
    3. Compute depth-aware weights on the survivor set only.
    4. If all survivor weights are zero (degenerate case), fall back to
       routing the full notional to the deepest surviving book.
    5. Otherwise, allocate = total_notional * w_i / sum(w).
    6. Return a new list in original order, with `allocated` filled for
       survivors and 0.0 for cut venues.

    The returned list preserves the input ordering so telemetry can align
    the pre- and post-filter views.

    Parameters
    ----------
    slots : Iterable[VenueSlot]
    total_notional : float
        The aggregate notional to distribute (in USD). Must be >= 0.
    delta : float
        Fractal impact index δ. Typically sourced from
        `strategy.fractal_delta` (not yet a separate module; see cost_model
        for the current interim δ). Must satisfy δ > -1 strictly.
    d_min : float
        Depth floor in USD. Defaults to DEFAULT_D_MIN_USD.

    Returns
    -------
    list[VenueSlot]
        New slots with `allocated` set.

    Raises
    ------
    ValueError
        On invalid inputs.
    """
    if total_notional < 0.0:
        raise ValueError("total_notional must be non-negative")
    if d_min < 0.0:
        raise ValueError("d_min must be non-negative")
    if delta <= -1.0:
        raise ValueError("delta must be > -1 (fractal exponent edge)")

    slot_list = list(slots)
    survivors_mask = [s.depth_usd >= d_min for s in slot_list]
    survivors = [s for s, keep in zip(slot_list, survivors_mask) if keep]

    # Case 1: nothing survives the cut → halt (all zeros).
    if not survivors or total_notional == 0.0:
        return [replace(s, allocated=0.0) for s in slot_list]

    weights = [_depth_aware_weight(s, delta) for s in survivors]
    wsum = sum(weights)

    if wsum <= 0.0:
        # Case 2: survivors exist but all weights underflowed → route
        # everything to the deepest surviving book (fail-safe fallback).
        deepest = max(survivors, key=lambda s: s.depth_usd)
        return [
            replace(s, allocated=(total_notional if s.venue == deepest.venue else 0.0))
            for s in slot_list
        ]

    # Normal case 3: weighted allocation across survivors.
    survivor_alloc: dict[str, float] = {}
    for s, w in zip(survivors, weights):
        survivor_alloc[s.venue] = total_notional * w / wsum

    return [
        replace(s, allocated=survivor_alloc.get(s.venue, 0.0))
        for s in slot_list
    ]


def cut_summary(slots: Iterable[VenueSlot], d_min: float = DEFAULT_D_MIN_USD) -> tuple[int, int, float]:
    """Return (n_cut, n_survive, total_cut_depth) for telemetry."""
    n_cut = 0
    n_survive = 0
    total_cut_depth = 0.0
    for s in slots:
        if s.depth_usd < d_min:
            n_cut += 1
            total_cut_depth += s.depth_usd
        else:
            n_survive += 1
    return n_cut, n_survive, total_cut_depth
