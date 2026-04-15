"""
fractal_delta.py — Aurora-Ω §6 Fractal Liquidity Dimension

Replace the fixed impact index δ = 0.35 with an OLS estimate of the
fractal slope ζ from the log-log depth curve:

    log D(Δp) = log C + ζ · log Δp + ε

then map to impact exponent

    δ = ζ / (1 + ζ).

Pure stdlib OLS (no numpy dependency). Returns a point estimate with a
standard error and R² so the caller can gate on model quality.

Spec: docs/aurora-omega-spec.md §6
Proof of consistency: §6.3 (plain OLS consistency under standard assumptions).

Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Sequence


# Fallback δ when regression is unusable (too few points, degenerate
# variance, NaN). This matches the pre-Aurora-Ω hardcoded 0.35 value in
# cost_model so the pipeline degrades gracefully.
FALLBACK_DELTA: float = 0.35

# Minimum sample count for a meaningful slope estimate. Three points are
# the algebraic minimum for variance; we require five to get a usable SE.
MIN_POINTS: int = 5

# Minimum R² for the fit to be considered trustworthy. Below this the
# caller should fall back to FALLBACK_DELTA.
MIN_R2: float = 0.60


@dataclass(frozen=True)
class FractalFit:
    """Output of a single depth-curve OLS fit.

    Attributes
    ----------
    zeta : float
        Slope estimate ẑ of log D vs log Δp.
    log_c : float
        Intercept (log of the scale constant C).
    delta : float
        Mapped impact exponent ẑ / (1 + ẑ). Clamped to [-0.5, 0.9] to
        stay inside the numerically stable range for the depth allocator.
    se_zeta : float
        Standard error of the slope estimate.
    r_squared : float
        Coefficient of determination.
    n : int
        Number of points used.
    trusted : bool
        True iff n >= MIN_POINTS AND r_squared >= MIN_R2 AND the mapped
        δ stayed inside the clamp bounds.
    """

    zeta: float
    log_c: float
    delta: float
    se_zeta: float
    r_squared: float
    n: int
    trusted: bool


def estimate_fractal_delta(
    delta_p: Sequence[float],
    depth: Sequence[float],
) -> FractalFit:
    """OLS log-log regression of depth vs price offset.

    Parameters
    ----------
    delta_p : Sequence[float]
        Price offsets in quote currency. Must be positive (log is taken).
    depth : Sequence[float]
        Observed depth at each offset. Must be positive.

    Returns
    -------
    FractalFit
    """
    n = min(len(delta_p), len(depth))
    if n < MIN_POINTS:
        return FractalFit(
            zeta=0.0, log_c=0.0, delta=FALLBACK_DELTA,
            se_zeta=float("inf"), r_squared=0.0, n=n, trusted=False,
        )

    xs: list[float] = []
    ys: list[float] = []
    for dp, d in zip(delta_p[:n], depth[:n]):
        if dp <= 0.0 or d <= 0.0:
            continue
        xs.append(math.log(dp))
        ys.append(math.log(d))

    n_used = len(xs)
    if n_used < MIN_POINTS:
        return FractalFit(
            zeta=0.0, log_c=0.0, delta=FALLBACK_DELTA,
            se_zeta=float("inf"), r_squared=0.0, n=n_used, trusted=False,
        )

    mean_x = sum(xs) / n_used
    mean_y = sum(ys) / n_used
    sxx = sum((x - mean_x) ** 2 for x in xs)
    sxy = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys))
    syy = sum((y - mean_y) ** 2 for y in ys)

    if sxx <= 0.0:
        return FractalFit(
            zeta=0.0, log_c=0.0, delta=FALLBACK_DELTA,
            se_zeta=float("inf"), r_squared=0.0, n=n_used, trusted=False,
        )

    zeta = sxy / sxx
    log_c = mean_y - zeta * mean_x

    # Residual variance → SE(ẑ). Gauss-Markov standard form:
    #     SE²(β̂) = σ² / Σ(x - x̄)²,   σ² = RSS / (n - 2)
    rss = sum(
        (y - (log_c + zeta * x)) ** 2 for x, y in zip(xs, ys)
    )
    if n_used > 2:
        sigma2 = rss / (n_used - 2)
        se_zeta = math.sqrt(max(sigma2 / sxx, 0.0))
    else:
        se_zeta = float("inf")

    r_squared = 1.0 - rss / syy if syy > 0 else 0.0

    # Map ẑ → δ̂ = ẑ / (1 + ẑ) and clamp.
    if zeta <= -1.0 + 1e-6:
        delta = FALLBACK_DELTA
        clamped = False
    else:
        raw_delta = zeta / (1.0 + zeta)
        delta = max(-0.5, min(0.9, raw_delta))
        clamped = (raw_delta < -0.5 or raw_delta > 0.9)

    trusted = (
        n_used >= MIN_POINTS
        and r_squared >= MIN_R2
        and not clamped
        and math.isfinite(se_zeta)
    )

    return FractalFit(
        zeta=zeta,
        log_c=log_c,
        delta=delta,
        se_zeta=se_zeta,
        r_squared=r_squared,
        n=n_used,
        trusted=trusted,
    )


def delta_or_fallback(fit: FractalFit) -> float:
    """Return fit.delta if trusted, else FALLBACK_DELTA."""
    return fit.delta if fit.trusted else FALLBACK_DELTA
