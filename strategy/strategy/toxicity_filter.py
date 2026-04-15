"""
toxicity_filter.py — Aurora-Ω §7, §9 Toxicity Filter Layer

Computes the linear toxicity score

    T_t = β₁ · r_t / σ_{m,t} + β₂ · S_t + β₃ · OBI_t + β₄ · ℓ_t

with default coefficients β = (0.6, 1.0, 0.5, 0.7). Converts the score
to a probability p_tox via a logistic link. Provides hooks for:

- label refinement (§9.1)
- queue-position feature (§9.2)
- adaptive β ridge re-fit on 10-min rolling AUC (§9.3)

The GBT ML layer in §7.2 is an offline-trained model the runtime calls
as a black box; this module exposes a TOXICITY_MODEL_INTERFACE Protocol
so the operator can plug in any scorer (offline GBT, online linear, or
a stub for paper trading). The default stub uses the pure linear score
through a logistic sigmoid.

Spec: docs/aurora-omega-spec.md §7, §8, §9
Pure stdlib.
"""
from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass, field
from typing import Protocol, Sequence


# Default β coefficients (§7.1).
DEFAULT_BETA: tuple[float, float, float, float] = (0.6, 1.0, 0.5, 0.7)

# Cancel threshold — quote is cancelled entirely when p_tox > 0.8 (§7.3).
CANCEL_P_TOX: float = 0.8

# Adaptive re-fit: rolling AUC window (10 min at 30-sec ticks = 20).
AUC_WINDOW: int = 20
AUC_REFIT_THRESHOLD: float = 0.75

# Adverse-loss bound (§8): r_* = 5e-4 s⁻¹.
R_STAR: float = 5e-4


# ---------------------------------------------------------------------------
# Feature record
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ToxFeatures:
    """Features fed into the toxicity score at one quote decision.

    Attributes
    ----------
    r_sigma : float
        Standardized mid drift r_t / σ_{m,t} (dimensionless).
    sweep : float
        Sweep event indicator (1.0 if a sweep occurred in the last window,
        else 0.0, or a continuous intensity).
    obi : float
        Order-book imbalance in [-1, 1].
    lead_lag : float
        Opposite-venue lead/lag signal (positive when the opposite venue
        is leading our side — suggests we are about to be picked off).
    queue_pos : float = 0.0
        §9.2 — our quote's queue position Q_ahead / Q_LOB in [0, 1].
        Higher = further back in the queue = higher adverse-selection risk.
    """

    r_sigma: float
    sweep: float
    obi: float
    lead_lag: float
    queue_pos: float = 0.0


# ---------------------------------------------------------------------------
# Scoring interface (operator-replaceable)
# ---------------------------------------------------------------------------


class ToxicityModel(Protocol):
    def score(self, features: ToxFeatures) -> float:
        """Return T_t (raw toxicity score)."""
        ...

    def probability(self, features: ToxFeatures) -> float:
        """Return p_tox ∈ [0, 1]."""
        ...


@dataclass
class LinearToxicityModel:
    """Default linear score + logistic probability. Ridge-refittable.

    T_t = Σ_i β_i · x_i (+ ε · queue_pos when enabled)

    p_tox = σ(k · (T_t - T_max))   — logistic with slope k and offset T_max.
    """

    beta: tuple[float, float, float, float] = DEFAULT_BETA
    queue_weight: float = 0.3
    t_max: float = 1.0
    logistic_slope: float = 2.5

    def score(self, f: ToxFeatures) -> float:
        b1, b2, b3, b4 = self.beta
        return (
            b1 * f.r_sigma
            + b2 * f.sweep
            + b3 * f.obi
            + b4 * f.lead_lag
            + self.queue_weight * f.queue_pos
        )

    def probability(self, f: ToxFeatures) -> float:
        t = self.score(f)
        z = self.logistic_slope * (t - self.t_max)
        if z >= 40:
            return 1.0
        if z <= -40:
            return 0.0
        return 1.0 / (1.0 + math.exp(-z))


# ---------------------------------------------------------------------------
# Decision output
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ToxicityDecision:
    p_tox: float
    score: float
    cancel: bool         # True → cancel the quote entirely
    offset_multiplier: float  # supplied to offset_controller as (1 + λ p^0.7)
    features: ToxFeatures


def evaluate(
    features: ToxFeatures,
    model: ToxicityModel,
    lam: float = 1.5,
) -> ToxicityDecision:
    """Main evaluation entry — called per quote decision."""
    t = model.score(features)
    p = model.probability(features)
    cancel = p > CANCEL_P_TOX
    # §7.3 offset multiplier component: (1 + λ · p^0.7).
    mult = 1.0 + lam * (p ** 0.7)
    return ToxicityDecision(
        p_tox=p, score=t, cancel=cancel,
        offset_multiplier=mult, features=features,
    )


# ---------------------------------------------------------------------------
# Adverse-loss bound (§8)
# ---------------------------------------------------------------------------


def adverse_loss_bound(
    phi: float,
    maker_notional: float,
    dt_s: float,
    r_star: float = R_STAR,
) -> float:
    """|PNL_adv| ≤ φ · Q · r_* · Δt.

    Returns the conservative upper bound on adverse-selection loss.
    Callers can use this as a real-time stop-loss: if empirical rolling
    loss exceeds this bound, something else is happening (not adverse
    selection).
    """
    if phi < 0 or maker_notional < 0 or dt_s < 0 or r_star < 0:
        return 0.0
    return phi * maker_notional * r_star * dt_s


# ---------------------------------------------------------------------------
# Rolling AUC tracker for §9.3 adaptive β refit
# ---------------------------------------------------------------------------


@dataclass
class LabeledObs:
    p_tox: float   # predicted probability at the time of the quote
    toxic: bool    # observed label (was the fill actually adverse?)


@dataclass
class AucTracker:
    """Fixed-window rolling AUC (Mann-Whitney U estimator).

    We compute AUC the classic way: probability a random positive has a
    higher predicted score than a random negative. For modest windows
    (20 obs) the naive O(n²) form is fine.
    """

    window: int = AUC_WINDOW
    obs: deque[LabeledObs] = field(default_factory=deque)

    def push(self, p_tox: float, toxic: bool) -> None:
        self.obs.append(LabeledObs(p_tox=p_tox, toxic=toxic))
        while len(self.obs) > self.window:
            self.obs.popleft()

    def auc(self) -> float:
        positives = [o.p_tox for o in self.obs if o.toxic]
        negatives = [o.p_tox for o in self.obs if not o.toxic]
        if not positives or not negatives:
            return 0.5
        wins = 0.0
        for p in positives:
            for n in negatives:
                if p > n:
                    wins += 1.0
                elif p == n:
                    wins += 0.5
        return wins / (len(positives) * len(negatives))

    def needs_refit(self) -> bool:
        if len(self.obs) < self.window:
            return False
        return self.auc() < AUC_REFIT_THRESHOLD


# ---------------------------------------------------------------------------
# Ridge re-fit of β on labeled observations (§9.3)
# ---------------------------------------------------------------------------


def ridge_refit_beta(
    features_list: Sequence[ToxFeatures],
    labels: Sequence[float],
    ridge_lambda: float = 0.1,
) -> tuple[float, float, float, float] | None:
    """Simple 4-feature ridge regression for β. Ignores queue_pos (§9.2 is
    handled as a fixed side-feature). Returns None if the design matrix
    is degenerate.

    Solves (XᵀX + λI) β = Xᵀy via 4×4 Gaussian elimination (stdlib).
    """
    n = min(len(features_list), len(labels))
    if n < 8:
        return None

    # Build XᵀX (4x4) and Xᵀy (4).
    xtx = [[0.0] * 4 for _ in range(4)]
    xty = [0.0] * 4
    for i in range(n):
        f = features_list[i]
        x = (f.r_sigma, f.sweep, f.obi, f.lead_lag)
        y = labels[i]
        for a in range(4):
            xty[a] += x[a] * y
            for b in range(4):
                xtx[a][b] += x[a] * x[b]

    for a in range(4):
        xtx[a][a] += ridge_lambda

    return _solve_4x4(xtx, xty)


def _solve_4x4(A: list[list[float]], b: list[float]) -> tuple[float, float, float, float] | None:
    """In-place 4x4 Gaussian elimination with partial pivoting."""
    M = [row[:] + [bi] for row, bi in zip(A, b)]
    for i in range(4):
        # Partial pivot
        max_row = i
        max_val = abs(M[i][i])
        for k in range(i + 1, 4):
            if abs(M[k][i]) > max_val:
                max_val = abs(M[k][i])
                max_row = k
        if max_val < 1e-12:
            return None
        M[i], M[max_row] = M[max_row], M[i]
        for k in range(i + 1, 4):
            factor = M[k][i] / M[i][i]
            for j in range(i, 5):
                M[k][j] -= factor * M[i][j]
    # Back-substitute
    x = [0.0] * 4
    for i in range(3, -1, -1):
        s = M[i][4]
        for j in range(i + 1, 4):
            s -= M[i][j] * x[j]
        x[i] = s / M[i][i]
    return (x[0], x[1], x[2], x[3])
