"""
funding_bandit.py — Aurora-Ω §19 UCB1 venue selector

Standard UCB1 bandit over hedge venues:

    UCB_k = r̄_k + sqrt(2 · log t / n_k)

Reward:

    r_k = funding_spread_gain_k - λ_s · expected_slippage_k

Regret: theoretical bound O(sqrt(K T log T)); empirical fit ≈ 0.19 sqrt(T)
(recorded as a telemetry observation, NOT a theorem — see §19.3).

Spec: docs/aurora-omega-spec.md §19
Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field


DEFAULT_EXPLORATION_C: float = math.sqrt(2.0)
DEFAULT_SLIPPAGE_WEIGHT: float = 1.0


@dataclass
class ArmStats:
    """Per-venue running stats for UCB1."""

    n: int = 0
    sum_reward: float = 0.0
    last_reward: float = 0.0

    def update(self, reward: float) -> None:
        self.n += 1
        self.sum_reward += reward
        self.last_reward = reward

    def mean(self) -> float:
        if self.n == 0:
            return 0.0
        return self.sum_reward / self.n


@dataclass
class BanditState:
    """Mutable state of the full bandit over a finite arm set.

    arms : dict[str, ArmStats]
        Per-venue stats. Arms are added on first reference.
    total_pulls : int
        Total plays across all arms (the `t` in UCB1).
    exploration_c : float
        Exploration constant in the UCB1 bound.
    slippage_weight : float
        λ_s in the reward formula.
    """

    arms: dict[str, ArmStats] = field(default_factory=dict)
    total_pulls: int = 0
    exploration_c: float = DEFAULT_EXPLORATION_C
    slippage_weight: float = DEFAULT_SLIPPAGE_WEIGHT

    def ensure(self, venue: str) -> ArmStats:
        if venue not in self.arms:
            self.arms[venue] = ArmStats()
        return self.arms[venue]

    def reward(self, funding_gain: float, expected_slippage: float) -> float:
        return funding_gain - self.slippage_weight * expected_slippage

    def ucb_score(self, venue: str) -> float:
        stats = self.arms.get(venue)
        if stats is None or stats.n == 0:
            return float("inf")  # unplayed arms always ranked first
        if self.total_pulls <= 0:
            return stats.mean()
        return stats.mean() + self.exploration_c * math.sqrt(
            math.log(max(self.total_pulls, 1)) / stats.n
        )

    def select(self, venues: list[str]) -> str:
        """Return the venue with the highest UCB score."""
        if not venues:
            raise ValueError("venues must be non-empty")
        for v in venues:
            self.ensure(v)
        best = max(venues, key=self.ucb_score)
        return best

    def observe(self, venue: str, funding_gain: float, expected_slippage: float) -> float:
        """Record a reward for the given venue and return the reward value."""
        r = self.reward(funding_gain, expected_slippage)
        stats = self.ensure(venue)
        stats.update(r)
        self.total_pulls += 1
        return r

    def snapshot(self) -> dict[str, dict[str, float]]:
        return {
            v: {"n": float(s.n), "mean_reward": s.mean(), "last_reward": s.last_reward}
            for v, s in self.arms.items()
        }


def empirical_regret_fit(t: int) -> float:
    """Aurora-Ω §19.3 empirical fit, NOT a theorem.

    empirical regret ≈ 0.19 · sqrt(t)

    Theoretical UCB1 bound is O(sqrt(K T log T)). This helper exists
    solely for telemetry — compare realized cumulative regret against
    this reference line to detect bandit degradation.
    """
    if t <= 0:
        return 0.0
    return 0.19 * math.sqrt(t)
