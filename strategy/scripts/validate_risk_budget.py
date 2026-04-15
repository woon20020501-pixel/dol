"""
validate_risk_budget.py — Appendix C CVaR budget LHS derivation

Replaces the provisional "CVaR_99 ≤ 76k" single-number bound with a
three-tier budget table derived from Latin Hypercube scenarios over the
Aurora-Ω cost model's parameter envelope.

Method (Appendix C):

  1. Parameter ranges — define plausible bounds for every calibration
     parameter that touches the tail (r*, fallback Exp rate, fallback
     Pareto α, Beta posterior (a, b), IOC target, toxicity frequency,
     per-tick position size).

  2. Latin Hypercube sampling — draw N scenarios from the product of
     those ranges. LHS gives better coverage than Monte Carlo at equal N.

  3. Per-scenario tail computation — for each scenario, simulate a
     synthetic loss distribution and compute CVaR_95 and CVaR_99.

  4. Aggregate the CVaR samples across scenarios and report:
     - median
     - 10th / 50th / 90th percentile
     - hi/lo band

  5. Derive budget tiers:
     halt    ≈ 90th percentile (conservative operational cap)
     warning ≈ median
     budget  ≈ 10th percentile (generous normal-operation threshold)

The exact numbers printed by this script are the Appendix C values. They
become the BudgetTable defaults in `strategy/risk_stack.py` until Phase 1
live data allows recalibration.

Run:
    PYTHONIOENCODING=utf-8 python scripts/validate_risk_budget.py
    PYTHONIOENCODING=utf-8 python scripts/validate_risk_budget.py --n-scenarios 100000 --n-losses 2000

Pure stdlib. LHS is implemented inline (no numpy/scipy).
"""
from __future__ import annotations

import argparse
import math
import random
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from strategy.risk_stack import cvar_ru, sample_std


# ---------------------------------------------------------------------------
# Parameter envelope for Appendix C scenarios
# ---------------------------------------------------------------------------
#
# These ranges describe the space of plausible regimes Aurora-Ω might
# encounter. Not all are independent — some co-vary in practice — but
# for a worst-case envelope we sample them independently (conservative).
#
# Every bound has an intended interpretation documented inline. When
# Phase 1 live data arrives, replace the synthetic bounds with empirical
# quantiles of the measured distributions.


@dataclass(frozen=True)
class ParameterEnvelope:
    """Portfolio-level envelope (not per-pair).

    Aurora-Ω typically runs ~46 active pairs at 50% of vault AUM. The
    notional range below represents the DEPLOYED portfolio size, and
    the simulator aggregates per-pair losses into a portfolio-level
    loss series via a shared basis-shock factor (PRINCIPLES §5.1).
    """

    # Deployed portfolio notional (total active, not per-pair)
    notional_min: float = 100_000.0
    notional_max: float = 1_500_000.0

    # Effective number of active pairs
    n_pairs_min: int = 20
    n_pairs_max: int = 46

    # Adverse-move drift bound per tick (high-probability r*)
    r_star_min: float = 1e-4
    r_star_max: float = 8e-4

    # Partial fill fraction from Beta(a, b)
    beta_a_min: float = 1.0
    beta_a_max: float = 5.0
    beta_b_min: float = 1.0
    beta_b_max: float = 10.0

    # Fallback mixture
    pareto_prob_min: float = 0.05
    pareto_prob_max: float = 0.15
    exp_mean_bps_min: float = 10.0
    exp_mean_bps_max: float = 35.0
    pareto_alpha_min: float = 2.1
    pareto_alpha_max: float = 4.5
    pareto_xmin_bps: float = 10.0

    # IOC failure frequency
    ioc_fail_prob_min: float = 0.02
    ioc_fail_prob_max: float = 0.08

    # Toxicity breach frequency
    tox_breach_prob_min: float = 0.01
    tox_breach_prob_max: float = 0.05

    # Basis-shock probability per tick — the dominant tail driver
    # (PRINCIPLES §5.1 Terra/FTX/JELLY regime). When it fires, fraction
    # `basis_corr_frac` of the portfolio gets hit by a `basis_shock_mag`
    # bad-mark shock simultaneously.
    basis_shock_prob_min: float = 0.0005  # ~1-2 per day at 1Hz
    basis_shock_prob_max: float = 0.003
    basis_corr_frac_min: float = 0.3      # 30% of pairs
    basis_corr_frac_max: float = 1.0      # 100% of pairs (single venue outage)
    basis_shock_mag_min: float = 0.02     # 2% bad mark
    basis_shock_mag_max: float = 0.20     # 20% bad mark

    # Tick observation window
    n_losses_per_scenario: int = 1000


# ---------------------------------------------------------------------------
# Latin Hypercube sampling (stdlib)
# ---------------------------------------------------------------------------


def latin_hypercube(n_samples: int, n_dims: int, rng: random.Random) -> list[list[float]]:
    """Classic LHS in [0, 1]^n_dims.

    Each dimension is split into n_samples equal strata, one sample per
    stratum; the strata are permuted independently per dimension so the
    resulting rows are space-filling without being a grid.
    """
    out: list[list[float]] = [[0.0] * n_dims for _ in range(n_samples)]
    for d in range(n_dims):
        # Jittered strata centers
        strata = [(i + rng.random()) / n_samples for i in range(n_samples)]
        rng.shuffle(strata)
        for i in range(n_samples):
            out[i][d] = strata[i]
    return out


def unit_to_range(u: float, lo: float, hi: float) -> float:
    return lo + u * (hi - lo)


# ---------------------------------------------------------------------------
# Synthetic loss generation per scenario
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ScenarioParams:
    notional: float
    n_pairs: int
    r_star: float
    beta_a: float
    beta_b: float
    pareto_prob: float
    exp_mean_bps: float
    pareto_alpha: float
    pareto_xmin_bps: float
    ioc_fail_prob: float
    tox_breach_prob: float
    basis_shock_prob: float
    basis_corr_frac: float
    basis_shock_mag: float


def materialize_scenario(u: list[float], env: ParameterEnvelope) -> ScenarioParams:
    """Map a 13-dim LHS unit vector to a concrete ScenarioParams."""
    return ScenarioParams(
        notional=unit_to_range(u[0], env.notional_min, env.notional_max),
        n_pairs=int(unit_to_range(u[1], env.n_pairs_min, env.n_pairs_max)),
        r_star=unit_to_range(u[2], env.r_star_min, env.r_star_max),
        beta_a=unit_to_range(u[3], env.beta_a_min, env.beta_a_max),
        beta_b=unit_to_range(u[4], env.beta_b_min, env.beta_b_max),
        pareto_prob=unit_to_range(u[5], env.pareto_prob_min, env.pareto_prob_max),
        exp_mean_bps=unit_to_range(u[6], env.exp_mean_bps_min, env.exp_mean_bps_max),
        pareto_alpha=unit_to_range(u[7], env.pareto_alpha_min, env.pareto_alpha_max),
        pareto_xmin_bps=env.pareto_xmin_bps,
        ioc_fail_prob=unit_to_range(u[8], env.ioc_fail_prob_min, env.ioc_fail_prob_max),
        tox_breach_prob=unit_to_range(u[9], env.tox_breach_prob_min, env.tox_breach_prob_max),
        basis_shock_prob=unit_to_range(u[10], env.basis_shock_prob_min, env.basis_shock_prob_max),
        basis_corr_frac=unit_to_range(u[11], env.basis_corr_frac_min, env.basis_corr_frac_max),
        basis_shock_mag=unit_to_range(u[12], env.basis_shock_mag_min, env.basis_shock_mag_max),
    )


def sample_fallback_bps(rng: random.Random, p: ScenarioParams) -> float:
    """Draw a fallback execution cost in bps from the Exp+Pareto mixture."""
    if rng.random() < p.pareto_prob:
        # Pareto: survival = (xmin / x)^α, inverse CDF
        u = rng.random()
        if u <= 0.0:
            u = 1e-12
        return p.pareto_xmin_bps * (1.0 - u) ** (-1.0 / p.pareto_alpha)
    else:
        u = rng.random()
        if u <= 0.0:
            u = 1e-12
        # Exp with mean = exp_mean_bps → rate = 1/mean
        return -math.log(u) * p.exp_mean_bps


def sample_phi(rng: random.Random, a: float, b: float) -> float:
    """Draw from Beta(a, b) via the gamma ratio trick (stdlib)."""
    x = rng.gammavariate(a, 1.0)
    y = rng.gammavariate(b, 1.0)
    if x + y <= 0.0:
        return 0.5
    return x / (x + y)


def simulate_scenario_losses(
    rng: random.Random,
    params: ScenarioParams,
    n_losses: int,
) -> list[float]:
    """Produce a synthetic PORTFOLIO loss series for a single scenario.

    Portfolio loss aggregates N_pairs independent per-pair losses plus a
    rare but large basis-blowout component that hits `basis_corr_frac`
    of the portfolio simultaneously with a `basis_shock_mag` bad mark.

    Per-tick loss = sum_{i=1..N} (per-pair routine loss)
                    + 1[basis shock fires] · basis_corr_frac · N · per_pair · mag
    """
    losses: list[float] = []
    dt_s = 1.0
    r_breach = 5e-3
    per_pair_notional = params.notional / max(params.n_pairs, 1)
    for _ in range(n_losses):
        tick_loss = 0.0
        # N_pairs independent per-pair routine losses
        for _p in range(params.n_pairs):
            phi = sample_phi(rng, params.beta_a, params.beta_b)
            loss_i = phi * per_pair_notional * params.r_star * dt_s
            if rng.random() < params.ioc_fail_prob:
                spread_bps = sample_fallback_bps(rng, params)
                loss_i += per_pair_notional * spread_bps * 1e-4
            if rng.random() < params.tox_breach_prob:
                loss_i += phi * per_pair_notional * r_breach
            tick_loss += loss_i
        # Correlated basis blowout (PRINCIPLES §5.1 tail)
        if rng.random() < params.basis_shock_prob:
            n_hit = int(params.n_pairs * params.basis_corr_frac)
            tick_loss += n_hit * per_pair_notional * params.basis_shock_mag
        losses.append(tick_loss)
    return losses


# ---------------------------------------------------------------------------
# Aggregate
# ---------------------------------------------------------------------------


@dataclass
class BudgetReport:
    n_scenarios: int
    alpha: float
    cvar_samples: list[float]

    def quantile(self, q: float) -> float:
        if not self.cvar_samples:
            return 0.0
        s = sorted(self.cvar_samples)
        idx = int(q * (len(s) - 1))
        return s[idx]

    def summarize(self) -> dict:
        return {
            "n_scenarios": self.n_scenarios,
            "alpha": self.alpha,
            "p50": self.quantile(0.50),
            "p75": self.quantile(0.75),
            "p85": self.quantile(0.85),
            "p90": self.quantile(0.90),
            "p95": self.quantile(0.95),
            "p99": self.quantile(0.99),
            "max": max(self.cvar_samples) if self.cvar_samples else 0.0,
            "std": sample_std(self.cvar_samples),
        }


def derive_budget(report: BudgetReport) -> dict:
    """Budget tier heuristics (Appendix C, corrected direction).

    The budget table represents LOSS thresholds where higher = worse:

        budget  threshold = median (p50)   — typical operating ceiling
        warning threshold = p85           — 85% of regimes stay below
        halt    threshold = p95           — catastrophic tail

    Above halt, the bot halts new orders. Between warning and halt, it
    de-risks aggressively (×0.3). Between budget and warning, it
    de-risks mildly (×0.5). Below budget, normal operation.
    """
    return {
        "budget": report.quantile(0.50),
        "warning": report.quantile(0.85),
        "halt": report.quantile(0.95),
        "alpha": report.alpha,
    }


def run_lhs(
    n_scenarios: int,
    n_losses: int,
    alpha: float,
    env: ParameterEnvelope,
    seed: int,
) -> BudgetReport:
    rng = random.Random(seed)
    lhs = latin_hypercube(n_scenarios, n_dims=13, rng=rng)
    cvars: list[float] = []
    for u in lhs:
        params = materialize_scenario(u, env)
        losses = simulate_scenario_losses(rng, params, n_losses)
        cvars.append(cvar_ru(losses, alpha=alpha))
    return BudgetReport(
        n_scenarios=n_scenarios,
        alpha=alpha,
        cvar_samples=cvars,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="CVaR budget Latin Hypercube derivation")
    parser.add_argument("--n-scenarios", type=int, default=5000,
                        help="number of LHS scenarios (default 5000, paper uses 1e6)")
    parser.add_argument("--n-losses", type=int, default=500,
                        help="losses simulated per scenario (default 500)")
    parser.add_argument("--seed", type=int, default=20260415)
    args = parser.parse_args()

    env = ParameterEnvelope()
    env = ParameterEnvelope(n_losses_per_scenario=args.n_losses)

    print("=" * 70)
    print("Aurora-Ω Appendix C — CVaR budget LHS derivation")
    print("=" * 70)
    print(f"Scenarios: {args.n_scenarios:,}")
    print(f"Losses per scenario: {args.n_losses:,}")
    print(f"Seed: {args.seed}")
    print()

    for alpha, label in [(0.95, "CVaR_95 (fast guard)"),
                         (0.99, "CVaR_99 (deep guard)")]:
        print(f"--- {label} ---")
        report = run_lhs(args.n_scenarios, args.n_losses, alpha, env, args.seed)
        summary = report.summarize()
        budget = derive_budget(report)
        print(f"  scenarios: {summary['n_scenarios']:,}")
        print(f"  p50:  ${summary['p50']:>12,.2f}")
        print(f"  p75:  ${summary['p75']:>12,.2f}")
        print(f"  p85:  ${summary['p85']:>12,.2f}")
        print(f"  p90:  ${summary['p90']:>12,.2f}")
        print(f"  p95:  ${summary['p95']:>12,.2f}")
        print(f"  p99:  ${summary['p99']:>12,.2f}")
        print(f"  max:  ${summary['max']:>12,.2f}")
        print()
        print(f"  → derived tiers:")
        print(f"      budget  (p50)   ≤ ${budget['budget']:>12,.0f}")
        print(f"      warning (p85)   ≤ ${budget['warning']:>12,.0f}")
        print(f"      halt    (p95)   ≤ ${budget['halt']:>12,.0f}")
        print()

    print("=" * 70)
    print("NOTE: provisional values. Re-run with --n-scenarios 1000000 for")
    print("production calibration. Phase 1 live data should replace the")
    print("parameter envelope with empirical quantiles.")
    print("=" * 70)
    return 0


if __name__ == "__main__":
    sys.exit(main())
