"""
risk_stack.py — Aurora-Ω §21 Risk Stack (4 layers)

Implements the four classical risk layers that feed the FSM:

  1. Entropic certainty equivalent
        CE_η(L) = (1/η) log E[exp(η L)]
     with small-η expansion CE ≈ E[L] + (η/2) Var(L). Under L = -log(1+R),
     E[L] is the negative Kelly growth rate.

  2. ECV (CVaR with std penalty, dimensionally clean):
        ECV_κ(L) = CVaR_{0.99}(L) + κ · Std(L)

  3. Rolling empirical CVaR via Rockafellar-Uryasev:
        CVaR_α(L) = min_c  c + (1/(1-α)) · E[(L - c)_+]

  4. Execution χ² goodness-of-fit: compare realized fill outcomes to the
     expected Beta(a,b) partial-fill distribution (or any expected density
     the caller provides).

Each layer exposes a simple `.evaluate(...)` that returns a RiskReport
with a bool `red_flag` used by the FSM (§22).

Spec: docs/aurora-omega-spec.md §21
Pure stdlib.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Sequence


# ---------------------------------------------------------------------------
# Common report type
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class RiskReport:
    layer: str
    value: float
    threshold: float
    red_flag: bool
    detail: str = ""


# ---------------------------------------------------------------------------
# Layer 1 — Entropic CE
# ---------------------------------------------------------------------------
#
# Donsker-Varadhan duality (robust-control interpretation):
#
#   CE_η(L) = (1/η) log E_P[exp(η L)]
#           = sup_Q { E_Q[L] - (1/η) KL(Q || P) }
#
# i.e., the Entropic CE equals the worst-case expected loss over all
# distributions Q that lie within a KL-divergence radius of 1/η around
# the model distribution P. This is the standard Fenchel-Legendre dual
# (Dupuis-Ellis 1997; Hansen-Sargent 2001 robust control).
#
# Operational interpretation for Aurora-Ω:
#   - Small η → large KL penalty coefficient (1/η) → Q confined tightly
#     near P → CE ≈ E_P[L] (risk-neutral; trust the model).
#   - Large η → small KL penalty → Q can drift further from P → CE is
#     the pessimistic expected loss under distributional perturbation.
#
# Thus the FSM mode-η mapping has a model-robustness meaning:
#   - Kelly-safe (η small) = "we trust the funding distribution"
#   - Robust    (η large) = "hedge against KL-ball worst case"
#
# Bounding CE_η(L) ≤ threshold is therefore EQUIVALENT to bounding the
# worst-case E_Q[L] over Q within KL-distance 1/η of P. This matches the
# Aurora-Ω §21.1 risk-stack intent: the Entropic CE layer protects the
# loss budget against model misspecification in funding drift, slippage
# distribution, and partial-fill posterior, up to the η-controlled KL
# ball. Spec cross-ref: `aurora-omega-spec.md` §21.1.


def entropic_ce(losses: Sequence[float], eta: float) -> float:
    """CE_η(L) = (1/η) log( (1/n) Σ exp(η L_i) ).

    Numerically stable via log-sum-exp. See module comment above for the
    Donsker-Varadhan dual interpretation (robust control over KL ball).
    """
    if not losses:
        return 0.0
    if eta == 0.0:
        return sum(losses) / len(losses)
    max_l = max(losses)
    # log-sum-exp trick for numerical stability
    sum_exp = sum(math.exp(eta * l - eta * max_l) for l in losses)
    return max_l + math.log(sum_exp / len(losses)) / eta


def entropic_ce_report(
    losses: Sequence[float],
    eta: float,
    threshold: float,
) -> RiskReport:
    ce = entropic_ce(losses, eta)
    return RiskReport(
        layer="entropic_ce",
        value=ce,
        threshold=threshold,
        red_flag=ce > threshold,
        detail=f"η={eta}, n={len(losses)}",
    )


# ---------------------------------------------------------------------------
# Layer 2 — ECV = CVaR + κ Std
# ---------------------------------------------------------------------------


def sample_std(xs: Sequence[float]) -> float:
    n = len(xs)
    if n < 2:
        return 0.0
    mu = sum(xs) / n
    var = sum((x - mu) ** 2 for x in xs) / (n - 1)
    return math.sqrt(max(var, 0.0))


def cvar_empirical(losses: Sequence[float], alpha: float) -> float:
    """Empirical CVaR at level α via the classic order-statistic method.

    CVaR_α(L) = average of the upper (1-α) tail of L.
    """
    if not losses:
        return 0.0
    if not (0.0 < alpha < 1.0):
        raise ValueError("alpha must be in (0, 1)")
    sorted_l = sorted(losses)
    n = len(sorted_l)
    k = int(math.ceil(alpha * n))
    tail = sorted_l[k:]
    if not tail:
        return sorted_l[-1]
    return sum(tail) / len(tail)


def ecv(losses: Sequence[float], kappa: float = 1.0, alpha: float = 0.99) -> float:
    return cvar_empirical(losses, alpha) + kappa * sample_std(losses)


def ecv_report(
    losses: Sequence[float],
    kappa: float,
    alpha: float,
    threshold: float,
) -> RiskReport:
    val = ecv(losses, kappa, alpha)
    return RiskReport(
        layer="ecv",
        value=val,
        threshold=threshold,
        red_flag=val > threshold,
        detail=f"α={alpha}, κ={kappa}",
    )


# ---------------------------------------------------------------------------
# Layer 3 — Rolling CVaR (Rockafellar-Uryasev form, same as layer 2's inner
# but exposed as a separate layer for telemetry)
# ---------------------------------------------------------------------------


def cvar_ru(losses: Sequence[float], alpha: float) -> float:
    """Rockafellar-Uryasev CVaR via grid search over c.

    CVaR_α(L) = min_c  c + (1/(1-α)) · mean((L - c)_+)

    For empirical distributions, the minimum is attained at the α-quantile,
    so we just return the quantile + tail-expectation form. Implemented
    redundantly with cvar_empirical to expose the R-U formulation for audit.
    """
    if not losses:
        return 0.0
    sorted_l = sorted(losses)
    n = len(sorted_l)
    q_idx = max(0, min(n - 1, int(math.ceil(alpha * n)) - 1))
    c = sorted_l[q_idx]
    slack = sum(max(l - c, 0.0) for l in losses) / n
    return c + slack / (1.0 - alpha)


def cvar_report(losses: Sequence[float], alpha: float, threshold: float) -> RiskReport:
    val = cvar_ru(losses, alpha)
    return RiskReport(
        layer="cvar",
        value=val,
        threshold=threshold,
        red_flag=val > threshold,
        detail=f"α={alpha}, n={len(losses)} (Rockafellar-Uryasev)",
    )


# ---------------------------------------------------------------------------
# Layer 4 — Execution χ²
# ---------------------------------------------------------------------------


def chi_square(
    observed: Sequence[float],
    expected: Sequence[float],
) -> float:
    """Pearson χ² goodness of fit: Σ (O_i - E_i)² / E_i."""
    if len(observed) != len(expected):
        raise ValueError("observed and expected length mismatch")
    total = 0.0
    for o, e in zip(observed, expected):
        if e <= 0:
            continue
        total += (o - e) ** 2 / e
    return total


def execution_chi2_report(
    observed: Sequence[float],
    expected: Sequence[float],
    threshold: float = 15.0,
) -> RiskReport:
    """Aurora-Ω §21.3: χ² > 15 triggers offset * 1.3."""
    val = chi_square(observed, expected)
    return RiskReport(
        layer="execution_chi2",
        value=val,
        threshold=threshold,
        red_flag=val > threshold,
        detail=f"bins={len(observed)}",
    )


# ---------------------------------------------------------------------------
# CVaR budget guard — Aurora-Ω §28 (post-review refactor)
# ---------------------------------------------------------------------------
#
# Replaces the single "provisional 76k" CVaR target with a three-tier budget
# table. Two independent guards operate at different quantiles:
#
#   (a) CVaR_95 fast guard    — frequently checked, tighter thresholds,
#                                reduces notional when loss distribution
#                                starts thickening.
#   (b) CVaR_99 deep guard    — catches true tail events, can halt trading.
#
# Both guards are driven by Latin-Hypercube scenarios in
# docs/math-aurora-omega-appendix.md Appendix C. The numbers below are
# PROVISIONAL pending Phase 1 live calibration; they must be reviewed
# before live capital.


@dataclass(frozen=True)
class BudgetTable:
    """Three-tier CVaR budget for operational risk control.

    Monotone tier structure: ``budget < warning < halt``.

    Semantics (losses are positive numbers, in USD):
      loss ≤ budget                  → normal operation, scale 1.0
      budget < loss ≤ warning        → de-risk, scale 0.5
      warning < loss ≤ halt          → emergency de-risk, scale 0.3
      loss > halt                    → halt new orders (scale 0.0)
    """

    budget: float
    warning: float
    halt: float
    alpha: float = 0.99

    def __post_init__(self) -> None:
        if not (self.budget < self.warning < self.halt):
            raise ValueError(
                f"BudgetTable must satisfy budget < warning < halt, got "
                f"({self.budget}, {self.warning}, {self.halt})"
            )
        if not (0.0 < self.alpha < 1.0):
            raise ValueError(f"alpha must be in (0, 1), got {self.alpha}")


# Default CVaR_99 deep budget — derived in math-aurora-omega-appendix.md
# Appendix C from a portfolio-level Latin Hypercube (100,000 scenarios,
# 400 losses each, 13-dim LHS with basis-blowout correlation). Tiers
# correspond to (p50, p85, p95) across the parameter envelope.
# PROVISIONAL — see Phase 1 calibration requirement. Scales linearly
# with portfolio notional; these values assume deployed notional of
# roughly $100k-$1.5M. Updated after the 100K run
# replaced the 2K-scenario bootstrap.
DEFAULT_BUDGET_99 = BudgetTable(
    budget=1_500.0,    # LHS p50 = $1,483
    warning=23_000.0,  # LHS p85 = $22,763
    halt=44_000.0,     # LHS p95 = $44,367
    alpha=0.99,
)

# Default CVaR_95 fast budget — same derivation, 0.95 level. The fast
# guard triggers earlier than the deep guard on the same loss stream.
DEFAULT_BUDGET_95 = BudgetTable(
    budget=1_000.0,    # LHS p50 = $965
    warning=5_100.0,   # LHS p85 = $5,093
    halt=9_500.0,      # LHS p95 = $9,490
    alpha=0.95,
)


@dataclass(frozen=True)
class GuardAction:
    """Output of a CVaR budget guard evaluation."""

    notional_scale: float
    halt: bool
    tier: str           # "ok" | "budget" | "warning" | "halt"
    cvar_value: float
    alpha: float


def cvar_guard(losses: Sequence[float], table: BudgetTable) -> GuardAction:
    """Evaluate rolling CVaR against a tiered budget.

    Returns a GuardAction telling the bot how to scale notional and
    whether to halt. The bot applies the returned scale multiplicatively
    to every open order; when halt=True, no new orders are submitted
    until the next tick's guard evaluation clears.

    This guard is independent of the FSM red-flag counter — it is a
    SECOND LINE OF DEFENSE that kicks in before the FSM's entropic CE
    or raw CVaR reports trigger, because tiered thresholds respond
    faster than 2-of-5 red flag voting.
    """
    val = cvar_ru(losses, alpha=table.alpha)
    if val <= table.budget:
        tier = "ok"
        scale = 1.0
        halt = False
    elif val <= table.warning:
        tier = "budget"
        scale = 0.5
        halt = False
    elif val <= table.halt:
        tier = "warning"
        scale = 0.3
        halt = False
    else:
        tier = "halt"
        scale = 0.0
        halt = True
    return GuardAction(
        notional_scale=scale,
        halt=halt,
        tier=tier,
        cvar_value=val,
        alpha=table.alpha,
    )
