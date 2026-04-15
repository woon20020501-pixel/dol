"""
partial_fill_model.py — Aurora-Ω §10, §11 Partial Fill Layer

Models the partial-fill fraction φ ∈ [0, 1] as Beta(a, b) with initial
prior Beta(2, 5). Bayesian update on observed fills. Dynamic q_min,
Kaplan-Meier residual survival, FIFO netting helpers.

Spec: docs/aurora-omega-spec.md §10, §11
Pure stdlib.
"""
from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass, field
from typing import Iterable


# §10.1 initial Beta prior
PRIOR_A: float = 2.0
PRIOR_B: float = 5.0

# §11.2 dynamic q_min formula: max(500, 20 * tickValue)
Q_MIN_FLOOR_USD: float = 500.0
Q_MIN_TICK_MULTIPLIER: float = 20.0

# §11.1 Kaplan survival cut — residual is force-flattened when S(t) < 0.05.
SURVIVAL_FLATTEN_THRESHOLD: float = 0.05


# ---------------------------------------------------------------------------
# Beta posterior
# ---------------------------------------------------------------------------


@dataclass
class BetaPosterior:
    """Mutable Beta(a, b) posterior for φ. Caller updates via update()."""

    a: float = PRIOR_A
    b: float = PRIOR_B

    def mean(self) -> float:
        s = self.a + self.b
        if s <= 0:
            return 0.5
        return self.a / s

    def variance(self) -> float:
        s = self.a + self.b
        if s <= 0:
            return 0.0
        return (self.a * self.b) / (s * s * (s + 1.0))

    def std(self) -> float:
        return math.sqrt(max(self.variance(), 0.0))

    def update(self, n_success: float, n_fail: float) -> None:
        """Conjugate update. n_success raises a; n_fail raises b.

        Fractional counts are allowed so the caller can weight events by
        recency or significance. Non-negative inputs required.
        """
        if n_success < 0 or n_fail < 0:
            raise ValueError("counts must be non-negative")
        self.a += n_success
        self.b += n_fail

    def decay(self, factor: float) -> None:
        """Exponential forgetting — pulls (a, b) toward the prior by
        `factor` ∈ [0, 1]. 0 = no forgetting, 1 = full reset to prior.

        Useful when the market regime changes and stale observations
        should be down-weighted.
        """
        if not (0.0 <= factor <= 1.0):
            raise ValueError("factor must be in [0, 1]")
        self.a = (1 - factor) * self.a + factor * PRIOR_A
        self.b = (1 - factor) * self.b + factor * PRIOR_B


# ---------------------------------------------------------------------------
# Dynamic q_min (§11.2)
# ---------------------------------------------------------------------------


def dynamic_q_min(tick_value_usd: float) -> float:
    """q_min = max(Q_MIN_FLOOR, 20 · tickValue)."""
    if tick_value_usd < 0:
        tick_value_usd = 0.0
    return max(Q_MIN_FLOOR_USD, Q_MIN_TICK_MULTIPLIER * tick_value_usd)


# ---------------------------------------------------------------------------
# Kaplan-Meier residual survival (§11.1)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SurvivalObs:
    """A single observation of residual maker quantity waiting to fill.

    duration_s : how long this residual existed before it either filled
                 (event=True) or was censored (event=False, e.g. cancelled).
    event : True if the residual was fully consumed; False if censored.
    """

    duration_s: float
    event: bool


def kaplan_meier_survival(
    observations: Iterable[SurvivalObs],
    t_query: float,
) -> float:
    """Classic Kaplan-Meier estimator at time t_query.

    S(t) = Π_{t_i ≤ t, event_i=True} (1 - d_i / n_i)

    where d_i is the number of events at t_i and n_i is the number at
    risk just before t_i.
    """
    obs = sorted(observations, key=lambda o: o.duration_s)
    n = len(obs)
    if n == 0:
        return 1.0

    s = 1.0
    at_risk = n
    i = 0
    while i < n:
        t = obs[i].duration_s
        if t > t_query:
            break
        # Tie group: all obs at time t
        events = 0
        group_n = 0
        j = i
        while j < n and obs[j].duration_s == t:
            if obs[j].event:
                events += 1
            group_n += 1
            j += 1
        if events > 0 and at_risk > 0:
            s *= (1.0 - events / at_risk)
        at_risk -= group_n
        i = j
    return s


def should_flatten_residual(
    observations: Iterable[SurvivalObs],
    elapsed_s: float,
    threshold: float = SURVIVAL_FLATTEN_THRESHOLD,
) -> bool:
    """True iff estimated S(elapsed) < threshold — force-flatten signal."""
    return kaplan_meier_survival(observations, elapsed_s) < threshold


# ---------------------------------------------------------------------------
# FIFO netting of residual fills within a cycle (§10.6)
# ---------------------------------------------------------------------------


@dataclass
class ResidualPool:
    """Aggregates partial-fill residuals within a funding cycle so the
    hedge leg is emitted in one batch rather than as many micro-orders.
    """

    pending: deque[tuple[int, float]] = field(default_factory=deque)
    # elements: (direction ∈ {-1, +1}, notional USD)

    def add(self, direction: int, notional: float) -> None:
        if direction not in (-1, +1):
            raise ValueError("direction must be ±1")
        if notional <= 0:
            return
        self.pending.append((direction, notional))

    def net(self) -> tuple[int, float]:
        """Returns (net_direction, net_notional) and clears the pool."""
        long = sum(n for d, n in self.pending if d == +1)
        short = sum(n for d, n in self.pending if d == -1)
        net_usd = long - short
        self.pending.clear()
        if net_usd == 0:
            return 0, 0.0
        direction = +1 if net_usd > 0 else -1
        return direction, abs(net_usd)

    def size(self) -> int:
        return len(self.pending)


# ---------------------------------------------------------------------------
# Hedge sizing given Beta posterior (§10.4, §10.5)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class HedgeDecision:
    """Output of size_hedge."""

    hedge_notional: float
    defer: bool           # True → accumulate into ResidualPool, don't send now
    reason: str


def size_hedge(
    maker_filled_notional: float,
    q_min: float,
    posterior: BetaPosterior,
    use_expected_phi: bool = False,
) -> HedgeDecision:
    """Compute hedge notional from actual maker fill.

    §10.4: q_h = φ · q_m. In the canonical form φ is the *observed* fill
    fraction, so the hedge equals the actual filled notional. With
    use_expected_phi=True the hedge size instead equals
    E[φ] · q_m using the Beta posterior mean — this is for predictive
    use cases (e.g. scheduling the IOC leg before the fill is final).

    §10.5: if hedge_notional < q_min, defer — either accumulate via
    ResidualPool or skip.
    """
    if use_expected_phi:
        phi = posterior.mean()
        q_h = phi * maker_filled_notional
    else:
        q_h = maker_filled_notional  # exact observed quantity

    if q_h < q_min:
        return HedgeDecision(hedge_notional=q_h, defer=True, reason="below_q_min")
    return HedgeDecision(hedge_notional=q_h, defer=False, reason="ok")
