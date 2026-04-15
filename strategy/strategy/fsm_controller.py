"""
fsm_controller.py — Aurora-Ω §22, §23 Fail-safe FSM + Kelly/Neutral/Robust

Five red-flag axes (4 risk layers + forecast scoring):

    [entropic_ce, ecv, cvar, execution_chi2, forecast]

When ≥ 2 flags fire:
- notional × 0.4
- emergency flatten timer 2 min
- retry budget reduced
- IOC window shrunk

Mode transitions:
    Kelly-safe  ← 0 red flags AND funding spread healthy AND forecast stable
    Neutral     ← 1 red flag OR forecast uncertain
    Robust      ← ≥ 2 red flags OR forecast deterioration OR execution χ² spike

Also implements the §24 Banach contraction parameter-adapter (returns a
damped θ_{t+1} from θ_t and the observed realized reward).

Spec: docs/aurora-omega-spec.md §22, §23, §24
Pure stdlib.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Sequence

from .risk_stack import RiskReport


class Mode(Enum):
    KELLY_SAFE = "kelly_safe"
    NEUTRAL = "neutral"
    ROBUST = "robust"


# Aurora-Ω §22 knobs
RED_FLAG_LIMIT: int = 2
NOTIONAL_SCALE_ROBUST: float = 0.4
EMERGENCY_FLATTEN_SECONDS: int = 120
RETRY_BUDGET_ROBUST: int = 1
IOC_WINDOW_ROBUST_MS: float = 80.0
IOC_WINDOW_NEUTRAL_MS: float = 140.0
IOC_WINDOW_KELLY_MS: float = 180.0


@dataclass(frozen=True)
class FsmDecision:
    mode: Mode
    red_flags_fired: tuple[str, ...]
    notional_scale: float
    emergency_flatten: bool
    emergency_flatten_seconds: int
    retry_budget: int
    ioc_window_ms: float
    rationale: str


@dataclass
class FsmState:
    """Mutable FSM state. Caller owns and passes it to step()."""

    mode: Mode = Mode.NEUTRAL
    last_flatten_at: float = 0.0


def _collect_flags(
    reports: Sequence[RiskReport],
    forecast_flag: bool,
) -> list[str]:
    fired = [r.layer for r in reports if r.red_flag]
    if forecast_flag:
        fired.append("forecast")
    return fired


def step(
    state: FsmState,
    now: float,
    reports: Sequence[RiskReport],
    forecast_flag: bool,
    funding_healthy: bool,
    cooldown_active: bool = False,
) -> FsmDecision:
    """Advance the FSM by one tick and return the action.

    `cooldown_active` — if True, the caller has an emergency_flatten timer
    running; the FSM sticks in Robust until the caller clears it.
    """
    fired = _collect_flags(reports, forecast_flag)
    n_fired = len(fired)

    # Robust takes precedence over everything else
    if n_fired >= RED_FLAG_LIMIT or cooldown_active:
        state.mode = Mode.ROBUST
        state.last_flatten_at = now
        return FsmDecision(
            mode=Mode.ROBUST,
            red_flags_fired=tuple(fired),
            notional_scale=NOTIONAL_SCALE_ROBUST,
            emergency_flatten=n_fired >= RED_FLAG_LIMIT,
            emergency_flatten_seconds=EMERGENCY_FLATTEN_SECONDS,
            retry_budget=RETRY_BUDGET_ROBUST,
            ioc_window_ms=IOC_WINDOW_ROBUST_MS,
            rationale=f"red_flags≥{RED_FLAG_LIMIT}: {fired}" if n_fired >= RED_FLAG_LIMIT else "cooldown_active",
        )

    # Single flag — Neutral with reduced aggression
    if n_fired == 1:
        state.mode = Mode.NEUTRAL
        return FsmDecision(
            mode=Mode.NEUTRAL,
            red_flags_fired=tuple(fired),
            notional_scale=0.75,
            emergency_flatten=False,
            emergency_flatten_seconds=0,
            retry_budget=2,
            ioc_window_ms=IOC_WINDOW_NEUTRAL_MS,
            rationale=f"1 red flag: {fired[0]}",
        )

    # Zero flags AND funding healthy — Kelly-safe
    if funding_healthy:
        state.mode = Mode.KELLY_SAFE
        return FsmDecision(
            mode=Mode.KELLY_SAFE,
            red_flags_fired=(),
            notional_scale=1.0,
            emergency_flatten=False,
            emergency_flatten_seconds=0,
            retry_budget=3,
            ioc_window_ms=IOC_WINDOW_KELLY_MS,
            rationale="all green + funding healthy",
        )

    # Zero flags but funding not healthy — Neutral baseline
    state.mode = Mode.NEUTRAL
    return FsmDecision(
        mode=Mode.NEUTRAL,
        red_flags_fired=(),
        notional_scale=0.85,
        emergency_flatten=False,
        emergency_flatten_seconds=0,
        retry_budget=2,
        ioc_window_ms=IOC_WINDOW_NEUTRAL_MS,
        rationale="clean but funding unhealthy",
    )


# ---------------------------------------------------------------------------
# §24 Self-correcting parameter map + hard-clip safeguard
# ---------------------------------------------------------------------------
#
# Review response #3 (2026-04-15): hard-clip and Banach contraction are
# TWO SEPARATE THINGS and must not be conflated.
#
#   - Banach contraction condition: T(θ) = (λ·E[R|θ] + β·u(θ))/(β+λ) is
#     a contraction with L_T = (λ·L_R + β·L_u)/(β+λ) < 1, where L_R and
#     L_u are Lipschitz constants of E[R|·] and u(·) in θ. This is a
#     PROPERTY OF THE MAP that must be *measured empirically* per
#     parameter (log δθ vs log t slope over a calibration window).
#
#   - Hard-clip safeguard: we enforce |θ_{t+1} − θ_t|_∞ ≤ max_step
#     regardless of whether T is a contraction. The clip guarantees
#     BOUNDED MOTION even when the Lipschitz condition fails. It is NOT
#     a proof of contraction.
#
# The bot runtime should log unclipped T(θ) values alongside clipped
# outputs so the operator can compute the empirical Lipschitz of T and
# confirm (or reject) the contraction hypothesis with real data.
#
# Default max_step=0.02 is an operational constant — the bot must
# re-calibrate once 24-48h of live θ history exists.

DEFAULT_MAX_STEP: float = 0.02


def self_correcting_update(
    theta: float,
    realized_reward: float,
    utility: float,
    lam: float,
    beta: float,
    max_step: float = DEFAULT_MAX_STEP,
) -> float:
    """Apply T(θ) with a hard-clip safeguard on the update step.

    Computes raw = (λ·E[R|θ_t] + β·u(θ_t)) / (β + λ), then returns
    θ_{t+1} = θ_t + clip(raw − θ_t, [−max_step, +max_step]).

    Parameters
    ----------
    theta : float
        Current parameter value θ_t.
    realized_reward : float
        Observed E[R | θ_t] for this tick.
    utility : float
        u(θ_t) — the operator's preference term.
    lam, beta : float
        Mixing weights. (β + λ) must be strictly positive.
    max_step : float
        Hard-clip bound on |θ_{t+1} − θ_t|. Default 0.02.

    Returns
    -------
    float
        Clipped θ_{t+1}.

    Notes
    -----
    This function does NOT verify the Banach contraction condition. The
    clip is a safeguard only; callers that need a formal convergence
    guarantee must independently measure L_T from log(|δθ_t|) regression
    as described in `docs/math-aurora-omega-appendix.md` Appendix B
    (Lemma S3 — empirical contraction measurement).
    """
    denom = beta + lam
    if denom <= 0:
        return theta
    if max_step < 0:
        raise ValueError("max_step must be non-negative")
    raw = (lam * realized_reward + beta * utility) / denom
    step = raw - theta
    if step > max_step:
        step = max_step
    elif step < -max_step:
        step = -max_step
    return theta + step


def empirical_lipschitz_estimate(
    theta_history: Sequence[float],
    t_history: Sequence[float],
    window: int = 50,
) -> float | None:
    """Empirical Lipschitz upper bound estimator for the self-correcting
    map, as sketched in Appendix B Lemma S3.

    We look at consecutive |δθ_t| = |θ_{t+1} − θ_t| over the last
    `window` ticks and take the maximum. This is a CRUDE upper bound
    (not a tight Lipschitz) but it's sufficient to decide whether the
    hard-clip is binding or not: if max |δθ| ≤ 0.5 × max_step
    throughout, the map is effectively unconstrained by the clip and
    the empirical Lipschitz is close to the unclipped T's Lipschitz.

    Returns None when there are fewer than 2 observations.
    """
    n = min(len(theta_history), len(t_history))
    if n < 2:
        return None
    start = max(0, n - window - 1)
    deltas = [
        abs(theta_history[i + 1] - theta_history[i])
        for i in range(start, n - 1)
    ]
    if not deltas:
        return None
    return max(deltas)
