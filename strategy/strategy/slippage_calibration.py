"""
slippage_calibration.py — Phase 1 recalibration hook for v3.5.2 cost_model

The v3.5.2 cost_model uses a square-root Almgren-Chriss impact estimator:

    slippage(Q) = clip(c · √(Q/D), [floor, ceiling])

where c = SLIPPAGE_IMPACT_COEFFICIENT is currently hardcoded in
`strategy/cost_model.py` as a conservative Phase 0 default (0.0008). Per
PRINCIPLES §2 there must be no backtest-derived fixed values in the
decision path; the v3.5.2 code explicitly acknowledges these five
constants as a narrowly-scoped exception pending Phase 1 live
calibration. This module closes that loop.

What this module does:

  1. Defines a SlippageObservation record the bot emits on every fill.
  2. Defines a SlippageCoefficients dataclass the bot can persist and
     pass into the cost model as a runtime override (instead of the
     module-level constants).
  3. Implements `recalibrate_impact_coefficient` — OLS-through-origin
     refit of the dominant c constant with truncation-aware filtering
     (floored and ceiling-hit observations are excluded because their
     realized_slippage is not the model's true output, it's a clip).
  4. Returns a `RecalibrationReport` the bot can log/audit. The report
     always includes enough information to audit acceptance or rejection
     decisions; the bot NEVER silently accepts a refit.

What this module does NOT do:

  - It does not refit the two depth-fraction constants (OI and VOL). Those
    have cross-correlated effects with the impact coefficient and require
    a 2D nonlinear fit; the hook leaves them at defaults until operator
    evidence shows they must change.
  - It does not refit the floor/ceiling. Those are mandate-set.
  - It does not persist the coefficients. That's the bot's responsibility
    via `persistence_store` (integration-spec §6).

Spec: `docs/aurora-omega-spec.md` §29, `docs/integration-spec.md` §6
calibration dependencies, `bot-implementation-matrix.md` §19.

Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, replace
from typing import Sequence

from .cost_model import (
    SLIPPAGE_CEILING,
    SLIPPAGE_FLOOR,
    SLIPPAGE_IMPACT_COEFFICIENT,
    SLIPPAGE_OI_FRACTION_AS_DEPTH,
    SLIPPAGE_VOL_FRACTION_AS_DEPTH,
)


# Acceptance-check defaults. The bot may override these per recalibration
# call but should document every deviation in the operator log.
MIN_RECAL_OBS: int = 30
MAX_RECAL_CHANGE_FACTOR: float = 3.0
MIN_RECAL_R_SQUARED: float = 0.20  # slippage is noisy; 0.2 is lenient


# ---------------------------------------------------------------------------
# Input / output records
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SlippageObservation:
    """A single observed fill used for calibration.

    Attributes
    ----------
    notional_usd : float
        The order notional at submission time.
    oi_usd : float
        Open interest on the venue at submission time (not at fill).
    vol_24h_usd : float
        Rolling 24h volume at submission time.
    realized_slippage : float
        Observed slippage as a fraction of notional. Computed by the bot
        as `abs(fill_price - expected_price) / expected_price` where
        `expected_price` is the framework's `p_star` from
        `fair_value_oracle.compute_fair_value` at submission time.
    ts : float
        Unix seconds of submission.
    """

    notional_usd: float
    oi_usd: float
    vol_24h_usd: float
    realized_slippage: float
    ts: float = 0.0


@dataclass(frozen=True)
class SlippageCoefficients:
    """Runtime-overridable slippage model constants.

    Defaults come from `strategy/cost_model.py`. The bot creates this via
    `SlippageCoefficients.defaults()`, mutates it via the recalibration
    hook, and passes the current snapshot into `slippage_with_coefficients`
    at decision time.
    """

    oi_fraction_as_depth: float
    vol_fraction_as_depth: float
    impact_coefficient: float
    floor: float
    ceiling: float

    @classmethod
    def defaults(cls) -> "SlippageCoefficients":
        return cls(
            oi_fraction_as_depth=SLIPPAGE_OI_FRACTION_AS_DEPTH,
            vol_fraction_as_depth=SLIPPAGE_VOL_FRACTION_AS_DEPTH,
            impact_coefficient=SLIPPAGE_IMPACT_COEFFICIENT,
            floor=SLIPPAGE_FLOOR,
            ceiling=SLIPPAGE_CEILING,
        )


@dataclass(frozen=True)
class RecalibrationReport:
    """Audit record for a recalibration attempt.

    Contains the raw OLS fit AND the acceptance decision. The bot MUST
    log this (not just the accept/reject bit) so the operator can see
    why a refit was rejected and how far the fit diverged from prior.
    """

    n_observations_total: int
    n_observations_used: int
    old_impact_coefficient: float
    new_impact_coefficient: float
    r_squared: float
    accepted: bool
    reason: str


# ---------------------------------------------------------------------------
# Runtime slippage evaluation with override-able coefficients
# ---------------------------------------------------------------------------


def _effective_depth(
    oi_usd: float,
    vol_24h_usd: float,
    coef: SlippageCoefficients,
) -> float:
    return max(
        coef.oi_fraction_as_depth * oi_usd,
        coef.vol_fraction_as_depth * vol_24h_usd,
        1_000.0,
    )


def slippage_with_coefficients(
    notional_usd: float,
    oi_usd: float,
    vol_24h_usd: float,
    coef: SlippageCoefficients,
) -> float:
    """Evaluate the slippage model with caller-supplied coefficients.

    This is the same formula as `cost_model.slippage` but reads the
    coefficients from a SlippageCoefficients instance instead of the
    module-level constants. The bot uses this after recalibration so the
    updated `impact_coefficient` takes effect without editing source.
    """
    if notional_usd <= 0.0:
        return 0.0
    depth = _effective_depth(oi_usd, vol_24h_usd, coef)
    raw = coef.impact_coefficient * math.sqrt(notional_usd / depth)
    return max(coef.floor, min(coef.ceiling, raw))


# ---------------------------------------------------------------------------
# OLS-through-origin recalibration
# ---------------------------------------------------------------------------


def _filter_observations(
    observations: Sequence[SlippageObservation],
    current: SlippageCoefficients,
) -> tuple[list[float], list[float]]:
    """Return (xs, ys) with xs = √(Q/D) and ys = realized_slippage for
    observations that are NOT truncated at floor or ceiling."""
    xs: list[float] = []
    ys: list[float] = []
    for obs in observations:
        if obs.notional_usd <= 0 or obs.realized_slippage <= 0:
            continue
        # Exclude truncation-biased observations
        if obs.realized_slippage <= current.floor * (1 + 1e-9):
            continue
        if obs.realized_slippage >= current.ceiling * (1 - 1e-9):
            continue
        depth = _effective_depth(obs.oi_usd, obs.vol_24h_usd, current)
        if depth <= 0:
            continue
        x = math.sqrt(obs.notional_usd / depth)
        if x <= 0:
            continue
        xs.append(x)
        ys.append(obs.realized_slippage)
    return xs, ys


def recalibrate_impact_coefficient(
    observations: Sequence[SlippageObservation],
    current: SlippageCoefficients,
    min_obs: int = MIN_RECAL_OBS,
    max_change_factor: float = MAX_RECAL_CHANGE_FACTOR,
    min_r_squared: float = MIN_RECAL_R_SQUARED,
) -> RecalibrationReport:
    """Refit SLIPPAGE_IMPACT_COEFFICIENT via OLS through the origin.

    Model: y = c · x   with x = √(Q/D), y = realized_slippage.
    Estimator: c_hat = Σ(x·y) / Σ(x²).
    Uncentered R²: 1 - Σ(y - c_hat·x)² / Σy².

    The refit is ACCEPTED only when:
      (i)   filtered observation count >= min_obs
      (ii)  the fitted c_hat is strictly positive
      (iii) c_hat / current.impact_coefficient is in [1/max_change_factor,
            max_change_factor] (bounded drift guard — a 10x jump in one
            recalibration almost always indicates a data pipeline bug)
      (iv)  uncentered R² >= min_r_squared (fit is above noise floor)

    On rejection, `new_impact_coefficient` in the report is the raw fit
    value (for audit) but the caller must NOT use it — call
    `apply_recalibration(current, report)` which honors the accepted bit.

    Parameters
    ----------
    observations : Sequence[SlippageObservation]
    current : SlippageCoefficients
        The currently-live coefficients.
    min_obs, max_change_factor, min_r_squared : float
        Override the default acceptance thresholds for this call.

    Returns
    -------
    RecalibrationReport
    """
    if min_obs < 2:
        raise ValueError("min_obs must be >= 2 for a meaningful fit")
    if max_change_factor <= 1.0:
        raise ValueError("max_change_factor must be > 1.0")
    if not (0.0 <= min_r_squared <= 1.0):
        raise ValueError("min_r_squared must be in [0, 1]")

    n_total = len(observations)
    xs, ys = _filter_observations(observations, current)
    n_used = len(xs)

    if n_used < min_obs:
        return RecalibrationReport(
            n_observations_total=n_total,
            n_observations_used=n_used,
            old_impact_coefficient=current.impact_coefficient,
            new_impact_coefficient=current.impact_coefficient,
            r_squared=0.0,
            accepted=False,
            reason=f"insufficient obs after filter: {n_used} < {min_obs}",
        )

    sxx = sum(x * x for x in xs)
    sxy = sum(x * y for x, y in zip(xs, ys))
    if sxx <= 0:
        return RecalibrationReport(
            n_observations_total=n_total,
            n_observations_used=n_used,
            old_impact_coefficient=current.impact_coefficient,
            new_impact_coefficient=current.impact_coefficient,
            r_squared=0.0,
            accepted=False,
            reason="degenerate Σx² = 0",
        )

    c_new = sxy / sxx
    rss = sum((y - c_new * x) ** 2 for x, y in zip(xs, ys))
    syy = sum(y * y for y in ys)
    r2 = 1.0 - rss / syy if syy > 0 else 0.0

    if c_new <= 0:
        return RecalibrationReport(
            n_observations_total=n_total,
            n_observations_used=n_used,
            old_impact_coefficient=current.impact_coefficient,
            new_impact_coefficient=c_new,
            r_squared=r2,
            accepted=False,
            reason=f"non-positive fit c_hat = {c_new:.6f}",
        )

    if current.impact_coefficient > 0:
        ratio = c_new / current.impact_coefficient
        if ratio > max_change_factor or ratio < 1.0 / max_change_factor:
            return RecalibrationReport(
                n_observations_total=n_total,
                n_observations_used=n_used,
                old_impact_coefficient=current.impact_coefficient,
                new_impact_coefficient=c_new,
                r_squared=r2,
                accepted=False,
                reason=(
                    f"change factor {ratio:.2f} outside "
                    f"[1/{max_change_factor}, {max_change_factor}]"
                ),
            )

    if r2 < min_r_squared:
        return RecalibrationReport(
            n_observations_total=n_total,
            n_observations_used=n_used,
            old_impact_coefficient=current.impact_coefficient,
            new_impact_coefficient=c_new,
            r_squared=r2,
            accepted=False,
            reason=f"R² {r2:.3f} < {min_r_squared}",
        )

    return RecalibrationReport(
        n_observations_total=n_total,
        n_observations_used=n_used,
        old_impact_coefficient=current.impact_coefficient,
        new_impact_coefficient=c_new,
        r_squared=r2,
        accepted=True,
        reason="ok",
    )


def apply_recalibration(
    current: SlippageCoefficients,
    report: RecalibrationReport,
) -> SlippageCoefficients:
    """Return updated coefficients if the report was accepted, else
    return the input unchanged.

    Callers should ALWAYS go through this function rather than mutating
    `impact_coefficient` directly, so the rejection path is honored
    consistently and there is no way to accidentally use an unvalidated
    fit.
    """
    if not report.accepted:
        return current
    return replace(current, impact_coefficient=report.new_impact_coefficient)
