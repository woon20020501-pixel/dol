"""
hedge_ioc.py — Aurora-Ω §12, §13 IOC hedge certainty layer

IOC success probability:

    P_IOC(τ, D) = sigmoid(-(τ - 100)/25) · (1 - exp(-D / 1.5e6))

Lower bound (§12.2):
    τ ≤ 200 ms AND D ≥ 2000  ⇒  P_IOC ≥ 0.65

Execution policy (§12.3, §13):
- IOC/FOK primary
- max hedge delay 50–150 ms
- latency outlier cut Z_τ > 3 ⇒ skip + defer
- retry backoff 80 ms → 140 ms → flatten
- fee-aware IOC-vs-join decision

Spec: docs/aurora-omega-spec.md §12, §13, §14
Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Sequence


# §12.1 parameters
TAU_PIVOT_MS: float = 100.0
TAU_SCALE_MS: float = 25.0
DEPTH_E_FOLD_USD: float = 1.5e6

# §12.2 minimum viable probability
MIN_P_IOC: float = 0.65

# §13.3 retry schedule (ms)
RETRY_SCHEDULE_MS: tuple[float, ...] = (80.0, 140.0)

# §13.1 latency outlier z-score cut
LATENCY_Z_CUT: float = 3.0


def p_ioc(tau_ms: float, depth_usd: float) -> float:
    """Aurora-Ω §12.1 formula."""
    if tau_ms < 0 or depth_usd < 0:
        return 0.0
    # Logistic component (decays as τ grows)
    z = (tau_ms - TAU_PIVOT_MS) / TAU_SCALE_MS
    if z >= 40:
        tau_term = 0.0
    elif z <= -40:
        tau_term = 1.0
    else:
        tau_term = 1.0 / (1.0 + math.exp(z))
    # Depth component (saturates as D grows)
    depth_term = 1.0 - math.exp(-depth_usd / DEPTH_E_FOLD_USD)
    return tau_term * depth_term


def viable(tau_ms: float, depth_usd: float, min_p: float = MIN_P_IOC) -> bool:
    return p_ioc(tau_ms, depth_usd) >= min_p


# ---------------------------------------------------------------------------
# Fee-aware IOC decision (§13.4)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class FeeProfile:
    maker_rebate_bps: float    # positive = rebate, negative = fee
    taker_fee_bps: float       # positive = fee
    slippage_bps: float        # expected slippage on the taker leg


def prefers_ioc(fp: FeeProfile) -> bool:
    """IOC is preferred when the net cost of taking beats the -2bp band
    relative to the maker rebate. Returns False → fall back to queue-join.
    """
    net_ioc_cost = -fp.maker_rebate_bps + fp.taker_fee_bps + fp.slippage_bps
    return net_ioc_cost <= 2.0  # accept up to +2bp net cost


# ---------------------------------------------------------------------------
# Latency outlier detection (§13.1)
# ---------------------------------------------------------------------------


@dataclass
class LatencyTracker:
    """Rolling mean/std of RTT samples for z-score detection."""

    window: int = 60
    samples: list[float] = field(default_factory=list)

    def push(self, rtt_ms: float) -> None:
        self.samples.append(rtt_ms)
        if len(self.samples) > self.window:
            self.samples.pop(0)

    def mean_std(self) -> tuple[float, float]:
        n = len(self.samples)
        if n == 0:
            return 0.0, 0.0
        mu = sum(self.samples) / n
        if n < 2:
            return mu, 0.0
        var = sum((x - mu) ** 2 for x in self.samples) / (n - 1)
        return mu, math.sqrt(max(var, 0.0))

    def z_score(self, rtt_ms: float) -> float:
        mu, sd = self.mean_std()
        if sd <= 0:
            return 0.0
        return (rtt_ms - mu) / sd

    def is_outlier(self, rtt_ms: float, cut: float = LATENCY_Z_CUT) -> bool:
        return self.z_score(rtt_ms) > cut


# ---------------------------------------------------------------------------
# Retry state machine (§13.3)
# ---------------------------------------------------------------------------


@dataclass
class RetryState:
    """Deterministic retry counter.

    step() is called after each failed IOC attempt and returns either
    the next backoff delay in ms or None to signal emergency flatten.
    """

    schedule_ms: tuple[float, ...] = RETRY_SCHEDULE_MS
    attempt: int = 0

    def step(self) -> float | None:
        if self.attempt >= len(self.schedule_ms):
            return None  # caller should emergency_flatten
        delay = self.schedule_ms[self.attempt]
        self.attempt += 1
        return delay

    def reset(self) -> None:
        self.attempt = 0


# ---------------------------------------------------------------------------
# Depth-aware failover ranking (§13.2)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class VenueHedgeCandidate:
    venue: str
    tau_ms: float
    depth_usd: float
    fee: FeeProfile


def failover_ranking(
    candidates: Sequence[VenueHedgeCandidate],
) -> list[VenueHedgeCandidate]:
    """Rank by D/τ (depth-latency tradeoff) descending. Filters out
    venues below viability threshold first.
    """
    viable_c = [c for c in candidates if viable(c.tau_ms, c.depth_usd)]

    def score(c: VenueHedgeCandidate) -> float:
        if c.tau_ms <= 0:
            return float("inf")
        return c.depth_usd / c.tau_ms

    viable_c.sort(key=score, reverse=True)
    return viable_c


def pick_primary(candidates: Sequence[VenueHedgeCandidate]) -> VenueHedgeCandidate | None:
    ranked = failover_ranking(candidates)
    return ranked[0] if ranked else None
