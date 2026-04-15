"""
fallback_router.py — Aurora-Ω §14 Fallback cost distribution + routing

Fallback spread modeled as Exp(λ) with mean 20 bp:

    ξ ~ Exp(λ),   λ = 1 / 0.0002
    C^fb = ξ · q_h

Plus routing policy: given a ranked list of hedge candidates (from
hedge_ioc.failover_ranking), pop the primary, and if it fails, move
down the list with a cost estimate per step.

Spec: docs/aurora-omega-spec.md §14, §13.2
Pure stdlib.
"""
from __future__ import annotations

import math
import random
from dataclasses import dataclass
from typing import Sequence

from .hedge_ioc import VenueHedgeCandidate


# §14.1 exponential rate
FALLBACK_MEAN_BPS: float = 20.0
FALLBACK_LAMBDA: float = 1.0 / (FALLBACK_MEAN_BPS * 1e-4)


def sample_fallback_spread_bps(rng: random.Random) -> float:
    """Draw a single sample of fallback spread in bps."""
    u = rng.random()
    if u <= 0.0:
        u = 1e-12
    return -math.log(u) / FALLBACK_LAMBDA / 1e-4  # bps


def expected_fallback_cost_usd(q_usd: float) -> float:
    """E[C^fb] = E[ξ] · q = 0.0020 · q."""
    return q_usd * FALLBACK_MEAN_BPS * 1e-4


def cvar_fallback_cost_usd(q_usd: float, alpha: float = 0.99) -> float:
    """For an exponential loss ξ, CVaR at level α is

        CVaR_α(ξ) = E[ξ] · (1 - log(1 - α))

    Returns the α-CVaR times q.
    """
    if not (0.0 < alpha < 1.0):
        raise ValueError("alpha must be in (0, 1)")
    multiplier = 1.0 - math.log(1.0 - alpha)
    return q_usd * FALLBACK_MEAN_BPS * 1e-4 * multiplier


# ---------------------------------------------------------------------------
# Routing policy
# ---------------------------------------------------------------------------


@dataclass
class Route:
    primary: VenueHedgeCandidate
    fallbacks: tuple[VenueHedgeCandidate, ...]
    total_candidates: int

    def next_fallback(self, failed: set[str]) -> VenueHedgeCandidate | None:
        for c in self.fallbacks:
            if c.venue not in failed:
                return c
        return None


def build_route(ranked: Sequence[VenueHedgeCandidate]) -> Route | None:
    if not ranked:
        return None
    return Route(
        primary=ranked[0],
        fallbacks=tuple(ranked[1:]),
        total_candidates=len(ranked),
    )
