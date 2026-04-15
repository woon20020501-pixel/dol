"""
funding_cycle_lock.py — Aurora-Ω §3.1 Funding Cycle Lock

Iron law linkage: the Dol strategy is a funding-capture engine. If the bot is
allowed to flip the funding-leg direction every minute, it is no longer a
funding harvester — it becomes a scalper paying round-trip fees without
receiving any funding. The cycle lock makes this failure mode structurally
impossible: within a single funding settlement window (default 3600s), the
leg direction h_c and target notional N_c are frozen. The execution leg
(maker→hedge micro layer) is still allowed to adjust *size* continuously
below N_c, and is allowed to CLOSE early (to release capital), but it cannot
FLIP direction without an explicit emergency_override signal from the FSM.

Spec: docs/aurora-omega-spec.md §3.1
Iron law: ../PRINCIPLES.md §1 (same-asset cross-venue funding hedge)

Pure stdlib; no external dependencies.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Optional


# Default funding settlement cadence. Pacifica is hourly; if a venue is on a
# different cadence (e.g. Hyperliquid 8h historically), the caller should pass
# the venue-specific period to the helpers below — we never hardcode the
# cadence into the decision logic.
DEFAULT_CYCLE_SECONDS: int = 3600

# Direction is integer-valued and restricted to {-1, 0, +1}. 0 means flat.
# Any other value is a programmer error and must abort.
VALID_DIRECTIONS = frozenset({-1, 0, 1})


@dataclass(frozen=True)
class CycleState:
    """Immutable snapshot of the current locked cycle.

    Attributes
    ----------
    cycle_index : int
        floor(opened_at / cycle_seconds) at the moment of opening.
    h_c : int
        Locked funding-leg direction in {-1, 0, +1}. +1 = long funding leg
        on Pacifica maker side; -1 = short. 0 = intentionally flat (lock
        still applies: you cannot start trading mid-cycle without unlock).
    N_c : float
        Locked target notional in USD (>= 0).
    opened_at : float
        Unix seconds when the cycle was opened. Used to compute time-to-close
        and to validate is_locked() after wall-clock drift.
    cycle_seconds : int
        The cadence this cycle was opened under. We store it so that a cycle
        opened under the Pacifica 3600s cadence is not accidentally checked
        against a different cadence on readback.
    """

    cycle_index: int
    h_c: int
    N_c: float
    opened_at: float
    cycle_seconds: int = DEFAULT_CYCLE_SECONDS


def cycle_index(t: float, cycle_seconds: int = DEFAULT_CYCLE_SECONDS) -> int:
    """Return the integer funding cycle index for unix time t."""
    if cycle_seconds <= 0:
        raise ValueError("cycle_seconds must be positive")
    return int(t // cycle_seconds)


def cycle_phase(t: float, cycle_seconds: int = DEFAULT_CYCLE_SECONDS) -> float:
    """Return the fraction of the current cycle that has elapsed (0..1)."""
    if cycle_seconds <= 0:
        raise ValueError("cycle_seconds must be positive")
    return (t % cycle_seconds) / cycle_seconds


def seconds_to_cycle_end(t: float, cycle_seconds: int = DEFAULT_CYCLE_SECONDS) -> float:
    """Return seconds remaining until the current cycle boundary."""
    return cycle_seconds - (t % cycle_seconds)


def is_locked(state: Optional[CycleState], now: float) -> bool:
    """True iff there is an open cycle and we are still inside it.

    If state is None (no cycle open), returns False. If the current time
    belongs to a *later* cycle index, the lock has expired and we return
    False — the caller is expected to close the old state and open a new
    one via open_cycle().
    """
    if state is None:
        return False
    return cycle_index(now, state.cycle_seconds) == state.cycle_index


def open_cycle(
    now: float,
    h_c: int,
    N_c: float,
    cycle_seconds: int = DEFAULT_CYCLE_SECONDS,
) -> CycleState:
    """Open a new locked cycle at time `now` with direction h_c and notional N_c.

    Raises
    ------
    ValueError
        If h_c is not in {-1, 0, +1} or N_c is negative.
    """
    if h_c not in VALID_DIRECTIONS:
        raise ValueError(f"h_c must be in {VALID_DIRECTIONS}, got {h_c!r}")
    if N_c < 0.0:
        raise ValueError(f"N_c must be non-negative, got {N_c}")
    return CycleState(
        cycle_index=cycle_index(now, cycle_seconds),
        h_c=h_c,
        N_c=N_c,
        opened_at=now,
        cycle_seconds=cycle_seconds,
    )


def enforce(
    state: Optional[CycleState],
    now: float,
    proposed_h: int,
    proposed_N: float,
    emergency_override: bool = False,
) -> tuple[int, float]:
    """Apply the cycle lock to a proposed (direction, notional) at time `now`.

    Semantics
    ---------
    Three cases, in priority order:

    1. `emergency_override=True` → proposed values pass through. This is the
       ONLY path by which an external component (FSM red-flag flatten, basis
       blowout detector, operator kill switch) can break the lock. Callers
       must log every override at WARN level.

    2. No lock (state is None or we have crossed a cycle boundary) → proposed
       values pass through. The caller is responsible for calling open_cycle()
       to re-lock at the start of the new cycle.

    3. Lock active → we return the LOCKED (h_c, N_c). The proposed direction
       is ignored (no flip). The proposed notional is also ignored — Aurora-Ω
       §3.1 states target_notional is frozen for the cycle. If the caller
       wants to de-risk *within* the cycle, it can close positions below N_c
       via the execution leg (which this function does not control); but the
       funding-leg TARGET stays at N_c.

    Returns
    -------
    (h_effective, N_effective) : tuple[int, float]
    """
    if proposed_h not in VALID_DIRECTIONS:
        raise ValueError(f"proposed_h must be in {VALID_DIRECTIONS}, got {proposed_h!r}")
    if proposed_N < 0.0:
        raise ValueError(f"proposed_N must be non-negative, got {proposed_N}")

    if emergency_override:
        return proposed_h, proposed_N
    if not is_locked(state, now):
        return proposed_h, proposed_N
    assert state is not None  # narrow for type-checker
    return state.h_c, state.N_c


def would_violate_lock(
    state: Optional[CycleState],
    now: float,
    proposed_h: int,
) -> bool:
    """Lightweight check: True if a *direction flip* attempt is about to be
    blocked by the lock (for logging / telemetry / RL reward shaping).

    A flip is a transition from a non-zero h_c to a different non-zero value
    OR to 0. Going from 0 to non-zero inside a locked cycle is also a
    violation (the lock says the cycle was opened flat and must stay flat
    until unlocked)."""
    if not is_locked(state, now):
        return False
    assert state is not None
    return proposed_h != state.h_c
