"""
forecast_scoring.py — Aurora-Ω §20 Forecast Scoring Layer (α-cascade)

Purpose: the bot contains several internal predictors — the fractal δ
estimator, the OU funding-spread model, the Kalman oracle lead/lag
tracker, the toxicity GBT, and the Beta-posterior partial-fill model.
When any of these predictors drifts away from the truth, the downstream
decisions degrade silently unless we have a principled monitor. This
module is that monitor.

We compute the α-cascade scoring rule

    S(x_hat, x) = -Σ_ℓ w_ℓ Σ_k |x_hat_k - x_k|^(α_0 + ℓ·η)

over predictor residuals, maintain a rolling baseline (mean + std) of
S_t, and emit a tail-estimate-deterioration red flag to the FSM when
the current score has fallen theta_S standard deviations below the
baseline.

The cascade is **strictly proper** for its per-coordinate joint
M-functional under the assumptions in Appendix F of
`docs/math-aurora-omega-appendix.md` (Aurora-Ω Proposition 7). A proof
sketch: each α-power loss is strictly consistent for its α-functional,
and a non-negative weighted sum of strictly proper rules is strictly
proper for the common functional (per-coordinate). See F.4 for the full
argument.

The cascade's endpoints recover the familiar limits:
    α = 1   → L1 loss (median-consistent, robust to outliers)
    α = 2   → squared error (mean-consistent)
    α ↑ ∞   → max-error (hard-threshold detector)

so the default grid α ∈ {1.0, 1.5, 2.0, 2.5, 3.0} with uniform weights
interpolates continuously from median-soft to upper-tail-hard — the
"L-1 / L-2 / L-3 limit modes" and "hard-threshold limit" unification
noted in the Aurora-Ω audit.

Spec: docs/aurora-omega-spec.md §20
Proof: docs/math-aurora-omega-appendix.md Appendix F

Pure stdlib; no external dependencies.
"""
from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass, field
from typing import Sequence


# ---------------------------------------------------------------------------
# Cascade configuration
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class CascadeConfig:
    """Parameters for the α-cascade scoring rule.

    Attributes
    ----------
    alpha_0 : float
        Lowest power in the cascade. Must be >= 1.0 for the strict-proper
        theorem to apply across the full cascade (F.1 assumption). The
        default 1.0 makes the lowest tier a median-consistent L1 loss.
    eta : float
        Step between successive powers in the cascade. Must be > 0. Default
        0.5 gives the grid {1.0, 1.5, 2.0, 2.5, 3.0} at L_max=4.
    L_max : int
        Cascade depth. Grid has L_max+1 entries: α_0, α_0+η, ..., α_0+L_max·η.
        Default 4 → 5-tier cascade.
    weights : tuple[float, ...]
        Non-negative weights, one per tier. Must sum to 1 and must have
        length L_max+1. Default is uniform 1/(L_max+1) on every tier.
    """

    alpha_0: float = 1.0
    eta: float = 0.5
    L_max: int = 4
    weights: tuple[float, ...] = ()

    def __post_init__(self) -> None:
        if self.alpha_0 < 1.0:
            raise ValueError(
                "alpha_0 must be >= 1.0 for the Aurora-Ω strict-proper theorem "
                "(see Appendix F.1). Subunit powers have non-convex losses."
            )
        if self.eta <= 0.0:
            raise ValueError("eta must be positive")
        if self.L_max < 0:
            raise ValueError("L_max must be non-negative")

        n_tiers = self.L_max + 1
        if not self.weights:
            uniform = tuple(1.0 / n_tiers for _ in range(n_tiers))
            # dataclass is frozen, so we bypass __setattr__
            object.__setattr__(self, "weights", uniform)

        if len(self.weights) != n_tiers:
            raise ValueError(
                f"weights length {len(self.weights)} != L_max+1 = {n_tiers}"
            )
        if any(w < 0 for w in self.weights):
            raise ValueError("weights must be non-negative")
        wsum = sum(self.weights)
        if abs(wsum - 1.0) > 1e-9:
            raise ValueError(f"weights must sum to 1, got {wsum}")
        if max(self.weights) == 0.0:
            raise ValueError("at least one weight must be > 0")

        # Require at least one tier with alpha > 1 and positive weight, so
        # the cascade has strict convexity (F.2 Lemma 1). If alpha_0 = 1
        # with all weight on tier 0, the "cascade" is pure L1 which is
        # convex but not strictly convex — Aurora-Ω §20 assumes a multi-
        # tier cascade, so we enforce the non-degeneracy here.
        has_strict = any(
            (self.alpha_0 + ell * self.eta) > 1.0 and w > 0
            for ell, w in enumerate(self.weights)
        )
        if not has_strict:
            raise ValueError(
                "cascade must have at least one tier with alpha > 1 and "
                "positive weight (see Appendix F.4 theorem assumptions)"
            )

    def alpha_grid(self) -> tuple[float, ...]:
        return tuple(self.alpha_0 + ell * self.eta for ell in range(self.L_max + 1))


# ---------------------------------------------------------------------------
# Scoring rule
# ---------------------------------------------------------------------------


def cascade_score(residuals: Sequence[float], cfg: CascadeConfig) -> float:
    """Compute S(x_hat, x) = -Σ_ℓ w_ℓ Σ_k |Δx_k|^(α_0 + ℓ·η).

    Negatively oriented: higher is better, -∞ is worst.

    Parameters
    ----------
    residuals : Sequence[float]
        Predictor errors Δx_k = x_hat_k - x_k (sign does not matter since
        we take absolute value). Pass all residuals from all predictors
        you want to score jointly; the cascade sums across them.
    cfg : CascadeConfig

    Returns
    -------
    float
        The scalar score. Always <= 0.
    """
    if not residuals:
        return 0.0

    alphas = cfg.alpha_grid()
    total = 0.0
    for w, a in zip(cfg.weights, alphas):
        if w == 0.0:
            continue
        inner = 0.0
        for r in residuals:
            inner += abs(r) ** a
        total += w * inner
    return -total


def cascade_score_components(
    residuals: Sequence[float],
    cfg: CascadeConfig,
) -> tuple[float, tuple[float, ...]]:
    """Return (total_score, per_tier_contributions).

    Useful for diagnosing which cascade tier is pushing the red flag.
    per_tier[ℓ] = -w_ℓ · Σ_k |Δx_k|^α_ℓ.
    """
    if not residuals:
        return 0.0, tuple(0.0 for _ in cfg.weights)

    alphas = cfg.alpha_grid()
    per_tier: list[float] = []
    total = 0.0
    for w, a in zip(cfg.weights, alphas):
        if w == 0.0:
            per_tier.append(0.0)
            continue
        inner = sum(abs(r) ** a for r in residuals)
        contribution = -(w * inner)
        per_tier.append(contribution)
        total += contribution
    return total, tuple(per_tier)


# ---------------------------------------------------------------------------
# Rolling baseline (mean, std) over the last W score observations
# ---------------------------------------------------------------------------


@dataclass
class BaselineRing:
    """Rolling buffer of recent scores for baseline estimation.

    We keep a simple fixed-size deque rather than a streaming Welford so
    the math stays auditable and the baseline matches what you'd get by
    hand from a recent log slice. The deque is bounded at `window`, and
    the caller pushes one score per tick.

    Attributes
    ----------
    window : int
        Maximum number of scores retained. For a 30-second tick cadence
        and a 30-minute baseline window, this is 60.
    scores : deque[float]
        Most recent scores, oldest at the left.
    """

    window: int = 60
    scores: deque[float] = field(default_factory=deque)

    def push(self, s: float) -> None:
        self.scores.append(s)
        while len(self.scores) > self.window:
            self.scores.popleft()

    def is_ready(self, min_samples: int = 10) -> bool:
        return len(self.scores) >= min_samples

    def mean_std(self) -> tuple[float, float]:
        """Return (mean, std) of the current buffer. (0.0, 0.0) if empty."""
        n = len(self.scores)
        if n == 0:
            return 0.0, 0.0
        mu = sum(self.scores) / n
        if n < 2:
            return mu, 0.0
        var = sum((x - mu) ** 2 for x in self.scores) / (n - 1)
        return mu, math.sqrt(max(var, 0.0))


# ---------------------------------------------------------------------------
# Red-flag trigger (Aurora-Ω §20.5)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class TailFlag:
    """Output of the tail-deterioration check.

    Attributes
    ----------
    fired : bool
        True iff the current score has dropped by >= theta_sigma baseline
        standard deviations below the baseline mean.
    delta : float
        current - baseline_mean. Negative when tails have worsened.
    z : float
        Standardized (current - mean) / std. NaN if std is zero.
    """

    fired: bool
    delta: float
    z: float


def tail_deterioration_flag(
    current: float,
    baseline: BaselineRing,
    theta_sigma: float = 2.0,
    min_samples: int = 10,
) -> TailFlag:
    """Evaluate the Aurora-Ω §20.5 tail-estimate deterioration trigger.

    Parameters
    ----------
    current : float
        The current cascade score S_t from cascade_score().
    baseline : BaselineRing
        Rolling baseline of recent S_t values. The CALLER is expected to
        NOT yet have pushed `current` to this ring — the baseline should
        represent the backdrop `current` is being compared against. After
        evaluating this flag, the caller should push `current` for the
        next tick's comparison.
    theta_sigma : float
        Number of baseline standard deviations below the baseline mean
        required to fire. Default 2.0 (Aurora-Ω §20.5).
    min_samples : int
        Minimum baseline buffer depth before the flag can fire. Prevents
        spurious fires at cold start.

    Returns
    -------
    TailFlag
    """
    if not baseline.is_ready(min_samples):
        return TailFlag(fired=False, delta=0.0, z=float("nan"))

    mu, sigma = baseline.mean_std()
    delta = current - mu
    if sigma <= 0.0:
        return TailFlag(fired=False, delta=delta, z=float("nan"))

    z = delta / sigma
    return TailFlag(fired=(z < -theta_sigma), delta=delta, z=z)
