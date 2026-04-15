"""
fair_value_oracle.py — Aurora-Ω §15-§17 Unified Fair-Value Oracle

Purpose: the bot reads mid-prices from 2-4 perp DEX venues, each with its
own mark convention, funding-reflection lag, quote age, and tick rounding.
Using any single venue's raw mid as the hedge reference price silently
poisons the neutrality of the same-asset hedge: when venue A's mark drifts
5 bps from venue B's mark, both legs look "correct" by their own book but
the combined position has a 5 bp directional exposure the bot is not
accounting for.

This module computes the unified reference price p* as

    p*(t) = Σ_k (χ_k(t) / Z(t)) · (m_k - F_k/8760 - δ_mark,k)

with exponential staleness weight χ_k(t) = exp(-(t - t_k)/τ_stale), a
hard drop for quotes that are either too old or too shallow, and optional
tick normalization + minimal 2-state Kalman lead/lag tracking.

Spec: docs/aurora-omega-spec.md §15, §16, §17
Iron law: ../PRINCIPLES.md §1 (hedge neutrality depends on a consistent fair
value across venues)

Pure stdlib; no external dependencies.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Iterable, Optional


# ---------------------------------------------------------------------------
# Constants (from Aurora-Ω §15-§17)
# ---------------------------------------------------------------------------

# Exponential staleness time-constant. A 1.5s-old quote retains weight
# e^(-1) ≈ 0.37; a 3s-old quote retains e^(-2) ≈ 0.14; at 4s we are just
# below the §17 "χ < 0.1 ⇒ bounded influence" threshold.
STALE_DECAY_TAU_SEC: float = 1.5

# Hard drop thresholds. A quote older than this OR shallower than this is
# dropped entirely (weight 0). This is §16.4 "Stale quote drop".
AGE_HARD_DROP_SEC: float = 5.0
DEPTH_HARD_DROP_USD: float = 1000.0

# §17 staleness bound: weight_k < 0.1 means stale quote influence is < 10%.
# We use this to assess whether the aggregate oracle is "healthy enough" to
# trust, by requiring that at least one venue contributes above this floor.
STALE_MIN_WEIGHT: float = 0.1

# Funding periods per year — annualized funding is divided by this to get
# per-period cost when correcting the mid. Pacifica is hourly, so 8760.
FUNDING_PERIODS_PER_YEAR: float = 8760.0


# ---------------------------------------------------------------------------
# Venue quote record
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class VenueQuote:
    """A single venue's snapshot at time `t_obs`.

    Attributes
    ----------
    venue : str
        Venue identifier (e.g. "pacifica", "hyperliquid").
    mid : float
        Mid price in quote currency (typically USD).
    t_obs : float
        Unix seconds when this quote was observed.
    depth_usd : float
        Effective book depth at the mid (USD-denominated). Used for the
        hard drop.
    funding_annual : float
        Current annualized funding rate (decimal; e.g. 0.12 = 12%/year).
        Used to adjust the mid toward fair by subtracting the per-period
        funding cost.
    mark_bias_bps : float
        Venue-specific mark bias in basis points, calibrated offline from
        cross-venue RMS. This is the "δ_mark,k" term in §15.1.
    tick_size : float
        Venue tick size in quote-currency units. 0.0 means unspecified.
    """

    venue: str
    mid: float
    t_obs: float
    depth_usd: float
    funding_annual: float = 0.0
    mark_bias_bps: float = 0.0
    tick_size: float = 0.0


# ---------------------------------------------------------------------------
# Core fair value computation
# ---------------------------------------------------------------------------


def staleness_weight(age_s: float, tau: float = STALE_DECAY_TAU_SEC) -> float:
    """χ_k(t) = exp(-(t - t_k)/τ)."""
    if age_s < 0.0:
        return 1.0  # clock skew: treat as fresh rather than inflating weight
    return math.exp(-age_s / tau)


def _drop_hard(age_s: float, depth_usd: float) -> bool:
    return age_s > AGE_HARD_DROP_SEC or depth_usd < DEPTH_HARD_DROP_USD


def _funding_adjusted_mid(q: VenueQuote) -> float:
    """m_k - F_k/8760 - δ_mark,k (§15.1 inner bracket).

    Funding is subtracted as a per-period absolute cost on the mid. The
    mark bias is subtracted in bps form. All three terms are in quote
    currency at the end.
    """
    per_period_funding = q.funding_annual / FUNDING_PERIODS_PER_YEAR
    mark_bias_abs = q.mark_bias_bps * 1e-4 * q.mid
    return q.mid - per_period_funding - mark_bias_abs


@dataclass(frozen=True)
class FairValue:
    """Oracle output at a single tick.

    Attributes
    ----------
    p_star : float
        Weighted fair value across all non-dropped venues.
    total_weight : float
        Sum of χ_k across contributing venues. 0.0 means all venues dropped.
    contributing_venues : tuple[str, ...]
        Venues that passed the hard drop filter. Useful for telemetry.
    healthy : bool
        True iff total_weight ≥ STALE_MIN_WEIGHT (i.e., at least one venue
        contributes above the §17 influence floor). Callers should halt
        order placement when healthy == False.
    """

    p_star: float
    total_weight: float
    contributing_venues: tuple[str, ...]
    healthy: bool


def compute_fair_value(
    quotes: Iterable[VenueQuote],
    now: float,
    tau: float = STALE_DECAY_TAU_SEC,
) -> FairValue:
    """Compute p* across a set of venue quotes at time `now`.

    Returns a degenerate FairValue(0, 0, (), False) if all venues drop.
    The caller must handle the healthy==False case (Aurora-Ω §17: halt
    trading rather than trust a single surviving stale quote).
    """
    num = 0.0
    den = 0.0
    kept: list[str] = []
    for q in quotes:
        age = now - q.t_obs
        if _drop_hard(age, q.depth_usd):
            continue
        w = staleness_weight(age, tau)
        num += w * _funding_adjusted_mid(q)
        den += w
        kept.append(q.venue)

    if den <= 0.0:
        return FairValue(
            p_star=0.0,
            total_weight=0.0,
            contributing_venues=(),
            healthy=False,
        )

    return FairValue(
        p_star=num / den,
        total_weight=den,
        contributing_venues=tuple(kept),
        healthy=den >= STALE_MIN_WEIGHT,
    )


# ---------------------------------------------------------------------------
# Tick normalization (§16.1)
# ---------------------------------------------------------------------------


def normalize_to_tick(price: float, tick_size: float) -> float:
    """Round `price` to the nearest multiple of `tick_size`.

    tick_size <= 0 returns the input unchanged (caller has no tick info).
    """
    if tick_size <= 0.0:
        return price
    return round(price / tick_size) * tick_size


# ---------------------------------------------------------------------------
# Minimal 2-state Kalman lead/lag tracker (§16.3)
# ---------------------------------------------------------------------------
#
# State vector: x = [price, drift]^T. Transition: price_{t+dt} = price_t +
# drift_t * dt + w_p;  drift_{t+dt} = drift_t + w_d. Observation: obs =
# price + v. This is the standard local-linear-trend model. We keep it
# minimal and audit-friendly rather than importing a Kalman library.


@dataclass
class Kalman2State:
    """Mutable Kalman filter state. Callers own it and pass it to step()."""

    p: float = 0.0          # current price estimate
    d: float = 0.0          # current drift estimate (price change per second)
    P00: float = 1.0        # Var(p)
    P01: float = 0.0        # Cov(p, d)
    P11: float = 1.0        # Var(d)

    # Process and observation noise — conservative defaults calibrated for
    # typical perp microstructure; re-tuned from live data during Phase 1.
    q_p: float = 1e-4
    q_d: float = 1e-6
    r: float = 1e-3


def kalman_init(initial_price: float) -> Kalman2State:
    return Kalman2State(p=initial_price, d=0.0)


def kalman_step(state: Kalman2State, obs: float, dt: float) -> None:
    """In-place Kalman predict+update for a single observation.

    dt is the time since the last step in seconds. obs is the new price
    observation (typically p* from compute_fair_value, but can also be a
    single venue's mid if the caller wants per-venue lead/lag).
    """
    if dt < 0.0:
        # Clock went backwards — skip this step rather than destabilize.
        return

    # Predict
    p_pred = state.p + state.d * dt
    d_pred = state.d
    # Covariance propagation for x_{t+dt} = F x_t + w, F = [[1, dt],[0, 1]]
    P00 = state.P00 + 2.0 * dt * state.P01 + dt * dt * state.P11 + state.q_p
    P01 = state.P01 + dt * state.P11
    P11 = state.P11 + state.q_d

    # Update (observation matrix H = [1, 0])
    S = P00 + state.r
    if S <= 0.0:
        return  # degenerate; skip
    K0 = P00 / S
    K1 = P01 / S
    y = obs - p_pred

    state.p = p_pred + K0 * y
    state.d = d_pred + K1 * y
    state.P00 = (1.0 - K0) * P00
    state.P01 = (1.0 - K0) * P01
    state.P11 = P11 - K1 * P01


# ---------------------------------------------------------------------------
# Funding clock shift (§16.2)
# ---------------------------------------------------------------------------


def clock_shift_correct(
    venue_ts: float,
    venue_api_lag_s: float,
) -> float:
    """Shift a venue's declared timestamp by its measured API lag.

    Some venues return `funding_updated_at` later than the actual on-chain
    settlement; we compensate by subtracting the measured lag. Negative lag
    means the venue is ahead of us (unusual — usually clock skew) and we
    treat it as zero.
    """
    return venue_ts - max(venue_api_lag_s, 0.0)
