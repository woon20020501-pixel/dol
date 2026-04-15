"""
generate_rust_fixtures.py — generates JSON fixtures from the Python
reference for use in Rust parity testing.

Output directory: `rust_fixtures/` (configurable).

Layout:
    rust_fixtures/
        phi.json
        ou_time_averaged_spread.json
        effective_spread_with_impact.json
        break_even_hold.json
        optimal_notional.json
        optimal_trading_contribution.json
        critical_aum.json
        bernstein_leverage.json
        mfg_competitor.json
        dol_sustainable_flow.json
        capacity_ceiling.json
        cap_routing.json
        mandate_floor.json
        slippage.json
        round_trip_cost_model_a.json
        round_trip_cost_model_c.json
        fit_ou_recovery.json
        adf_type1.json
        adf_power.json
        cvar_drawdown.json
        hurst_dfa.json
        expected_residual_income.json
        dry_run_end_to_end.json

Each file is a list of test cases:
    [
      {
        "name": "phi_at_zero",
        "input": {"x": 0.0},
        "expected": {"result": 1.0},
        "tolerance": 1e-15,
        "notes": "phi(0) by definition = 1 (Taylor limit)"
      },
      ...
    ]

The Rust side loads the same JSON, iterates each case, and asserts the
expected value is within the stated tolerance.

Functions that correspond 1:1 with the Python reference are imported
directly. v4 extensions (MFG, Bernstein, phi, etc.) provide reference
implementations inside this script.

Usage:
    python scripts/generate_rust_fixtures.py
    python scripts/generate_rust_fixtures.py --output /path/to/rust_fixtures
    python scripts/generate_rust_fixtures.py --section phi,ou
"""
import argparse
import json
import math
import os
import random
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from strategy.cost_model import (
    Mandate,
    LiveInputs,
    slippage as cost_slippage,
    round_trip_cost_pct,
    target_vault_apy,
    target_vault_apy_floor,
    lifecycle_annualized_return,
)
from strategy.stochastic import (
    fit_ou,
    fit_drift,
    adf_test,
    expected_residual_income,
    cvar_drawdown_stop,
    lower_tail_mean,
    upper_tail_mean,
    _generate_ou_sample,
)
from strategy.portfolio import (
    _norm_inv,
    covariance_matrix,
    shrink_covariance,
)
from strategy.rigorous import (
    required_leverage_rigorous,
)
from strategy.frontier import (
    hurst_dfa,
    empirical_bernstein_radius,
    conformal_interval,
)


# =============================================================================
# v4 extension reference implementations (for Rust parity)
# =============================================================================
# The functions below are not present in the Python framework package; they
# come from the v4 math design document. This script is the single source of
# truth for them.


def phi_reference(x: float) -> float:
    """φ(x) = (1 - e^(-x))/x, with φ(0) := 1.

    Numerically stable: uses `math.expm1` to preserve precision near zero.
    Naive form `(1 - math.exp(-x))/x` suffers catastrophic cancellation
    for |x| ≲ 1e-8, producing errors ~1e-5 relative. See .

    At exactly x = 0, returns 1 (limit).
    For very small |x|, falls back to Taylor series for robustness even
    if a future libm misbehaves on subnormal expm1.

    Rust parity: use `f64::exp_m1(-x)` or (1 − `(-x).exp_m1()`)/x.
    the bot team Item 1 resolution (2026-04-14).
    """
    if x == 0.0:
        return 1.0
    if abs(x) < 1e-8:
        # φ(x) = 1 - x/2 + x²/6 - x³/24 + O(x⁴)
        return 1.0 - x / 2.0 + x * x / 6.0 - x * x * x / 24.0
    # expm1(-x) = e^(-x) - 1, so 1 - e^(-x) = -expm1(-x)
    return -math.expm1(-x) / x


def phi_derivative_reference(x: float) -> float:
    """φ'(x) = [(1+x)e^(-x) - 1] / x²

    Near zero: φ'(x) = -1/2 + x/3 - x²/8 + x³/30 - O(x⁴).
    The naive form has two cancellations ((1+x)e^(-x) vs 1, then divide by x²),
    so Taylor fallback is used for small |x|.

    Rust parity: use same branch. For |x| ≥ 1e-4, formula is stable.
    """
    if x == 0.0:
        return -0.5
    if abs(x) < 1e-4:
        # Taylor: -1/2 + x/3 - x²/8 + x³/30 - O(x⁴)
        return -0.5 + x / 3.0 - x * x / 8.0 + x * x * x / 30.0
    return ((1.0 + x) * math.exp(-x) - 1.0) / (x * x)


def ou_time_averaged_spread(d0_annual, mu_annual, theta_ou_per_h, tau_h):
    """D̄(τ; D₀) = μ̃ + (D₀ - μ̃) · φ(θ^OU · τ)

    (math reference §D.2.
    """
    x = theta_ou_per_h * tau_h
    return mu_annual + (d0_annual - mu_annual) * phi_reference(x)


def effective_spread_with_impact(
    d0_annual, mu_annual, theta_ou_per_h, tau_h,
    n_per_leg, pi_pac, theta_impact, rho_comp,
):
    """D̄(τ; D₀, n) = (1 - θ^impact n/Π) / (1 + ρ^comp)
                      × [μ̃ + (D₀-μ̃)·φ(θ^OU·τ)]

    (math reference §D.3.
    """
    ou_avg = ou_time_averaged_spread(d0_annual, mu_annual, theta_ou_per_h, tau_h)
    impact_factor = 1.0 - theta_impact * n_per_leg / pi_pac
    if impact_factor <= 0.0:
        return 0.0
    comp_factor = 1.0 / (1.0 + rho_comp)
    return ou_avg * impact_factor * comp_factor


def break_even_hold_at_mean(mu_annual, c_round_trip, rho_comp):
    """τ^BE = 8760 · c · (1 + ρ^comp) / μ̃  (when D₀ = μ̃)

    (math reference §D.4.
    """
    if mu_annual <= 0.0:
        return float("inf")
    return 8760.0 * c_round_trip * (1.0 + rho_comp) / mu_annual


def break_even_hold_fixed_point(
    d0_annual, mu_annual, theta_ou_per_h, c_round_trip, rho_comp,
    initial_tau_h=168.0, max_iter=100, tol=1e-6,
):
    """Fixed-point iteration for τ^BE when D₀ ≠ μ."""
    tau = initial_tau_h
    for _ in range(max_iter):
        d_eff = ou_time_averaged_spread(d0_annual, mu_annual, theta_ou_per_h, tau)
        if d_eff <= 0.0:
            return None
        tau_new = 8760.0 * c_round_trip * (1.0 + rho_comp) / d_eff
        if abs(tau_new - tau) < tol:
            return tau_new
        tau = tau_new
    return None  # did not converge


def optimal_margin_fraction(tau_be_h, tau_h, gamma_i):
    """w_i^⋆ = (1 - τ^BE/τ) / (2 γ_i).  (math reference §D.5."""
    factor = 1.0 - tau_be_h / tau_h
    if factor <= 0.0 or gamma_i <= 0.0:
        return 0.0
    return factor / (2.0 * gamma_i)


def optimal_notional(pi_pac, tau_be_h, tau_h, theta_impact):
    """n_i^⋆ = Π (1 - τ^BE/τ) / (2 θ^impact).  (math reference §D.5."""
    factor = 1.0 - tau_be_h / tau_h
    if factor <= 0.0 or theta_impact <= 0.0:
        return 0.0
    return pi_pac * factor / (2.0 * theta_impact)


def optimal_trading_contribution(
    d_eff_annual, pi_pac, rho_comp, theta_impact, aum, tau_be_h, tau_h,
):
    """T_i^⋆ = D^eff · Π / [4·(1+ρ)·θ^impact·A] · (1 - τ^BE/τ)².

    L-independent. (math reference §D.5.
    """
    if aum <= 0.0 or theta_impact <= 0.0:
        return 0.0
    factor = 1.0 - tau_be_h / tau_h
    if factor <= 0.0:
        return 0.0
    return (
        d_eff_annual * pi_pac * factor * factor
        / (4.0 * (1.0 + rho_comp) * theta_impact * aum)
    )


def critical_aum(pi_pac, tau_be_h, tau_h, theta_impact, leverage, m_pos):
    """A^crit_i = Π (1 - τ^BE/τ) / (θ^impact · L · m_pos).  Part D.6."""
    factor = 1.0 - tau_be_h / tau_h
    if factor <= 0.0 or m_pos <= 0.0:
        return float("inf")
    return pi_pac * factor / (theta_impact * leverage * m_pos)


def bernstein_leverage_bound(mmr, delta_per_h, sigma_per_h, tau_h, epsilon):
    """L^R(τ) = [MMR + Δ·L_ε/3 + √((Δ·L_ε/3)² + 2·τ·σ²·L_ε)]^(-1).

    L_ε = ln(1/ε). Integer floor returned.
    (math reference §D.7.
    """
    if epsilon <= 0.0 or epsilon >= 1.0:
        return None
    l_eps = math.log(1.0 / epsilon)
    delta_term = delta_per_h * l_eps / 3.0
    var_term = 2.0 * tau_h * sigma_per_h * sigma_per_h * l_eps
    sqrt_term = math.sqrt(delta_term * delta_term + var_term)
    y_star = delta_term + sqrt_term
    l_cont = 1.0 / (mmr + y_star)
    if l_cont < 1.0:
        return 1
    return int(math.floor(l_cont))


def mfg_competitor_count(pi_pac, d_eff_annual, theta_impact, c_op_marginal):
    """K^* = √(Π·D^eff / (θ^impact·C_op)) - 1.  Part D.8."""
    if c_op_marginal <= 0.0 or theta_impact <= 0.0:
        return None
    ratio = pi_pac * d_eff_annual / (theta_impact * c_op_marginal)
    if ratio < 0.0:
        return 0.0
    return max(math.sqrt(ratio) - 1.0, 0.0)


def dol_sustainable_flow_per_pair(c_op_marginal, c_op_dol):
    """V^Dol_flow = C_op^marginal - C_op^Dol (must be positive)."""
    if c_op_marginal <= c_op_dol:
        return None
    return c_op_marginal - c_op_dol


def capacity_ceiling(n_active_pairs, delta_c_op, r_floor, alpha_min, r_idle):
    """A^* = N · ΔC_op / (R_floor - α_min·r_idle).  Part D.8."""
    denominator = r_floor - alpha_min * r_idle
    if denominator <= 0.0:
        return None
    return n_active_pairs * delta_c_op / denominator


def cap_routing_reference(vault_gross, cut_c, cut_b, cut_r, cust_max, buf_max):
    """Part D.9. Returns (customer, buffer, reserve)."""
    cust_raw = vault_gross * cut_c
    buf_raw = vault_gross * cut_b
    res_raw = vault_gross * cut_r

    cust = max(0.0, min(cust_raw, cust_max))
    excess_1 = max(0.0, cust_raw - cust_max)
    buf_with_excess = buf_raw + excess_1
    buf = max(0.0, min(buf_with_excess, buf_max))
    excess_2 = max(0.0, buf_with_excess - buf_max)
    res = res_raw + excess_2
    return cust, buf, res


def mandate_floor_reference(cust_min, cut_c, buf_min, cut_b):
    """R_floor = max(cust_min/cut_c, buf_min/cut_b).  Part D.9."""
    return max(cust_min / cut_c, buf_min / cut_b)


def round_trip_cost_model_c(
    phi_m_p, phi_t_p, phi_t_c,
    slip_p, slip_c,
    bridge_rt,
    legging_window_seconds, sigma_price_per_sqrt_day,
):
    """Model C hybrid execution cost.  (math reference §D.10.

    fee   = (φ_m^{v_p} + φ_t^{v_p}) + 2·φ_t^{v_c}
    slip  = σ(n, Π_p) + 2·σ(n, Π_c)   # pivot assumed maker-filled (1 taker fallback)
    ε_leg = σ_price × √t_leg / √(2π), × 2 for round trip
    """
    fee = (phi_m_p + phi_t_p) + 2.0 * phi_t_c
    slip = slip_p + 2.0 * slip_c
    sigma_per_sqrt_sec = sigma_price_per_sqrt_day / math.sqrt(86400.0)
    epsilon_leg = (
        sigma_per_sqrt_sec * math.sqrt(legging_window_seconds)
        / math.sqrt(2.0 * math.pi)
    )
    legging = 2.0 * epsilon_leg
    return fee + slip + legging + bridge_rt


def round_trip_cost_model_a(phi_t_p, phi_t_c, slip_p, slip_c, bridge_rt):
    """Model A: both legs taker, round trip.  Baseline for Rust comparison."""
    return 2.0 * phi_t_p + 2.0 * phi_t_c + 2.0 * slip_p + 2.0 * slip_c + bridge_rt


# =============================================================================
# Fixture container utility
# =============================================================================


def write_fixtures(path: str, cases: list):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(cases, f, indent=2, ensure_ascii=False, default=_jsonify)


def _jsonify(obj):
    if isinstance(obj, float) and (math.isinf(obj) or math.isnan(obj)):
        return str(obj)
    if isinstance(obj, (list, tuple)):
        return [_jsonify(x) for x in obj]
    return str(obj)


def make_case(name: str, input_dict: dict, expected_dict: dict,
              tolerance: float, notes: str = "") -> dict:
    return {
        "name": name,
        "input": _sanitize(input_dict),
        "expected": _sanitize(expected_dict),
        "tolerance": tolerance,
        "notes": notes,
    }


def _sanitize(d: dict) -> dict:
    """Float inf/nan → string for JSON compatibility."""
    out = {}
    for k, v in d.items():
        if isinstance(v, float):
            if math.isinf(v):
                out[k] = "inf" if v > 0 else "-inf"
            elif math.isnan(v):
                out[k] = "nan"
            else:
                out[k] = v
        elif isinstance(v, (list, tuple)):
            out[k] = [_sanitize_atom(x) for x in v]
        else:
            out[k] = v
    return out


def _sanitize_atom(v):
    if isinstance(v, float):
        if math.isinf(v):
            return "inf" if v > 0 else "-inf"
        if math.isnan(v):
            return "nan"
    return v


# =============================================================================
# Section generators
# =============================================================================

# --- §1. phi function -----------------------------------------------------

def gen_phi(output_dir):
    cases = []
    # Exact identities
    cases.append(make_case("phi_at_zero_exact",
                           {"x": 0.0}, {"result": 1.0},
                           1e-15, "φ(0) = 1 by Taylor limit"))
    # Small positive x (Taylor regime)
    for x in [1e-15, 1e-12, 1e-10, 1e-8, 1e-6, 1e-4, 1e-3, 1e-2]:
        cases.append(make_case(f"phi_small_{x}",
                               {"x": x}, {"result": phi_reference(x)},
                               1e-13, "Taylor-regime"))
    # Typical operating range (x ~ 0.1 to 10)
    for x in [0.1, 0.3, 0.5, 0.7, 1.0, 1.5, 2.0, 3.0, 5.0, 7.0, 10.0]:
        cases.append(make_case(f"phi_mid_{x}",
                               {"x": x}, {"result": phi_reference(x)},
                               1e-12, "Normal range"))
    # Large x (φ → 0)
    for x in [20.0, 30.0, 50.0, 100.0, 200.0, 500.0]:
        cases.append(make_case(f"phi_large_{x}",
                               {"x": x}, {"result": phi_reference(x)},
                               1e-12, "Large x, φ → 0"))
    # Monotonicity sanity: φ(x_i) > φ(x_{i+1})
    xs = [0.1, 0.5, 1.0, 2.0, 5.0, 10.0]
    values = [phi_reference(x) for x in xs]
    cases.append(make_case("phi_monotone_decreasing",
                           {"xs": xs},
                           {"values": values, "all_decreasing": True},
                           1e-12, "Property test"))

    # Phi derivative values
    for x in [0.0, 0.1, 1.0, 5.0, 10.0]:
        cases.append(make_case(f"phi_derivative_{x}",
                               {"x": x}, {"result": phi_derivative_reference(x)},
                               1e-10, "φ'(x) for verification"))

    write_fixtures(f"{output_dir}/phi.json", cases)
    return len(cases)


# --- §2. OU time-averaged spread ------------------------------------------

def gen_ou_time_averaged_spread(output_dir):
    cases = []

    # At τ → 0: D̄ → D₀ (no reversion time)
    cases.append(make_case(
        "ou_tau_zero_returns_d0",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 1e-10},
        {"result": 0.15}, 1e-6,
        "τ ≈ 0 → receive D₀"
    ))
    # At τ → ∞: D̄ → μ (fully reverted)
    cases.append(make_case(
        "ou_tau_infty_returns_mu",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 1e6},
        {"result": ou_time_averaged_spread(0.15, 0.10, 0.01, 1e6)},
        1e-10, "τ → ∞, D̄ → μ"
    ))
    # D₀ = μ: invariant in τ
    for tau_h in [1.0, 100.0, 720.0, 8760.0]:
        cases.append(make_case(
            f"ou_d0_eq_mu_{tau_h}",
            {"d0": 0.10, "mu": 0.10, "theta_ou": 0.01, "tau_h": tau_h},
            {"result": 0.10}, 1e-12,
            "D₀ = μ → D̄ = μ regardless of τ"
        ))
    # Typical operating cases
    typical = [
        (0.15, 0.10, 0.01, 168.0),   # 1 week hold
        (0.15, 0.10, 0.01, 720.0),   # 1 month hold
        (0.20, 0.08, 0.02, 336.0),   # stronger mean reversion
        (0.05, 0.10, 0.005, 168.0),  # below mean entry
        (-0.10, 0.00, 0.01, 168.0),  # negative spread
    ]
    for d0, mu, th, tau in typical:
        cases.append(make_case(
            f"ou_typical_d0_{d0}_mu_{mu}_th_{th}_tau_{tau}",
            {"d0": d0, "mu": mu, "theta_ou": th, "tau_h": tau},
            {"result": ou_time_averaged_spread(d0, mu, th, tau)},
            1e-12,
            f"D₀={d0}, μ={mu}, θ^OU={th}, τ={tau}h"
        ))

    write_fixtures(f"{output_dir}/ou_time_averaged_spread.json", cases)
    return len(cases)


# --- §3. Effective spread with impact -------------------------------------

def gen_effective_spread_with_impact(output_dir):
    cases = []
    # n = 0: no impact, reduces to pure OU average
    cases.append(make_case(
        "impact_n_zero",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 168.0,
         "n_per_leg": 0.0, "pi_pac": 5_000_000.0,
         "theta_impact": 0.5, "rho_comp": 0.0},
        {"result": effective_spread_with_impact(
            0.15, 0.10, 0.01, 168.0, 0.0, 5_000_000.0, 0.5, 0.0)},
        1e-12, "n=0 → no impact"
    ))
    # rho = 0: solo trader
    cases.append(make_case(
        "impact_rho_zero",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 168.0,
         "n_per_leg": 50_000.0, "pi_pac": 5_000_000.0,
         "theta_impact": 0.5, "rho_comp": 0.0},
        {"result": effective_spread_with_impact(
            0.15, 0.10, 0.01, 168.0, 50_000.0, 5_000_000.0, 0.5, 0.0)},
        1e-12, "Solo trader"
    ))
    # High competition
    cases.append(make_case(
        "impact_high_competition",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 168.0,
         "n_per_leg": 50_000.0, "pi_pac": 5_000_000.0,
         "theta_impact": 0.5, "rho_comp": 10.0},
        {"result": effective_spread_with_impact(
            0.15, 0.10, 0.01, 168.0, 50_000.0, 5_000_000.0, 0.5, 10.0)},
        1e-12, "10 competitors → /11 discount"
    ))
    # Impact factor negative → returns 0
    cases.append(make_case(
        "impact_saturated_to_zero",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01, "tau_h": 168.0,
         "n_per_leg": 20_000_000.0, "pi_pac": 5_000_000.0,
         "theta_impact": 0.5, "rho_comp": 0.0},
        {"result": 0.0}, 1e-12,
        "n·θ/Π > 1 → signal saturated, result = 0"
    ))
    # Typical operation
    cases.append(make_case(
        "impact_typical",
        {"d0": 0.12, "mu": 0.08, "theta_ou": 0.005, "tau_h": 336.0,
         "n_per_leg": 100_000.0, "pi_pac": 5_000_000.0,
         "theta_impact": 0.5, "rho_comp": 1.0},
        {"result": effective_spread_with_impact(
            0.12, 0.08, 0.005, 336.0, 100_000.0, 5_000_000.0, 0.5, 1.0)},
        1e-12, "Typical 2-week hold"
    ))

    write_fixtures(f"{output_dir}/effective_spread_with_impact.json", cases)
    return len(cases)


# --- §4. Break-even hold --------------------------------------------------

def gen_break_even_hold(output_dir):
    cases = []
    # At mean: closed form
    cases.append(make_case(
        "be_at_mean_typical",
        {"mu": 0.10, "c_round_trip": 0.002, "rho_comp": 0.0},
        {"result": break_even_hold_at_mean(0.10, 0.002, 0.0)},
        1e-12, "τ^BE = 8760·c/μ"
    ))
    cases.append(make_case(
        "be_at_mean_with_competition",
        {"mu": 0.10, "c_round_trip": 0.002, "rho_comp": 2.0},
        {"result": break_even_hold_at_mean(0.10, 0.002, 2.0)},
        1e-12, "3× longer with 2 competitors"
    ))
    # Low mu → long break-even
    cases.append(make_case(
        "be_low_mu",
        {"mu": 0.01, "c_round_trip": 0.002, "rho_comp": 0.0},
        {"result": break_even_hold_at_mean(0.01, 0.002, 0.0)},
        1e-12, "Low spread → long τ^BE"
    ))
    # Fixed point for D₀ ≠ μ
    fp = break_even_hold_fixed_point(
        d0_annual=0.15, mu_annual=0.10, theta_ou_per_h=0.01,
        c_round_trip=0.002, rho_comp=1.0,
    )
    cases.append(make_case(
        "be_fixed_point_above_mean",
        {"d0": 0.15, "mu": 0.10, "theta_ou": 0.01,
         "c_round_trip": 0.002, "rho_comp": 1.0},
        {"result": fp}, 1e-4,
        "Fixed-point iteration, D₀ > μ"
    ))

    write_fixtures(f"{output_dir}/break_even_hold.json", cases)
    return len(cases)


# --- §5. Optimal notional + contribution ----------------------------------

def gen_optimal_notional(output_dir):
    cases = []
    # Break-even → n = 0
    cases.append(make_case(
        "n_star_at_breakeven",
        {"pi_pac": 5_000_000.0, "tau_be_h": 168.0, "tau_h": 168.0,
         "theta_impact": 0.5},
        {"result": 0.0}, 1e-12,
        "τ = τ^BE → n_star = 0"
    ))
    # τ → ∞: upper bound n_star = Π/(2θ)
    cases.append(make_case(
        "n_star_upper_bound",
        {"pi_pac": 5_000_000.0, "tau_be_h": 168.0, "tau_h": 1e10,
         "theta_impact": 0.5},
        {"result": optimal_notional(5_000_000.0, 168.0, 1e10, 0.5)},
        1e-3,
        "τ → ∞, n_star → Π/(2θ)"
    ))
    # Typical
    for tau_h in [168.0, 336.0, 720.0]:
        cases.append(make_case(
            f"n_star_typical_tau_{tau_h}",
            {"pi_pac": 5_000_000.0, "tau_be_h": 50.0, "tau_h": tau_h,
             "theta_impact": 0.5},
            {"result": optimal_notional(5_000_000.0, 50.0, tau_h, 0.5)},
            1e-9, f"Typical τ = {tau_h}h"
        ))

    write_fixtures(f"{output_dir}/optimal_notional.json", cases)
    return len(cases)


def gen_optimal_trading_contribution(output_dir):
    cases = []
    # Break-even → 0
    cases.append(make_case(
        "T_star_at_breakeven_zero",
        {"d_eff": 0.10, "pi_pac": 5_000_000.0, "rho_comp": 0.0,
         "theta_impact": 0.5, "aum": 1_000_000.0,
         "tau_be_h": 168.0, "tau_h": 168.0},
        {"result": 0.0}, 1e-12, "τ = τ^BE → T* = 0"
    ))
    # Typical cases
    typical = [
        (0.10, 5_000_000.0, 0.0, 0.5, 1_000_000.0, 50.0, 168.0),
        (0.10, 5_000_000.0, 1.0, 0.5, 1_000_000.0, 50.0, 168.0),
        (0.15, 5_000_000.0, 0.5, 0.5, 10_000_000.0, 70.0, 336.0),
        (0.08, 10_000_000.0, 2.0, 0.3, 5_000_000.0, 100.0, 720.0),
    ]
    for d_eff, pi, rho, th, a, t_be, t in typical:
        cases.append(make_case(
            f"T_star_d_{d_eff}_a_{a}",
            {"d_eff": d_eff, "pi_pac": pi, "rho_comp": rho,
             "theta_impact": th, "aum": a,
             "tau_be_h": t_be, "tau_h": t},
            {"result": optimal_trading_contribution(
                d_eff, pi, rho, th, a, t_be, t)},
            1e-12, "Typical trading contribution"
        ))

    write_fixtures(f"{output_dir}/optimal_trading_contribution.json", cases)
    return len(cases)


# --- §6. Critical AUM -----------------------------------------------------

def gen_critical_aum(output_dir):
    cases = []
    # Small AUM: far below critical
    cases.append(make_case(
        "crit_aum_typical_leverage_2",
        {"pi_pac": 5_000_000.0, "tau_be_h": 50.0, "tau_h": 168.0,
         "theta_impact": 0.5, "leverage": 2, "m_pos": 0.02},
        {"result": critical_aum(5_000_000.0, 50.0, 168.0, 0.5, 2, 0.02)},
        1e-6, "L=2 critical AUM"
    ))
    cases.append(make_case(
        "crit_aum_higher_leverage",
        {"pi_pac": 5_000_000.0, "tau_be_h": 50.0, "tau_h": 168.0,
         "theta_impact": 0.5, "leverage": 3, "m_pos": 0.02},
        {"result": critical_aum(5_000_000.0, 50.0, 168.0, 0.5, 3, 0.02)},
        1e-6, "L=3 (smaller)"
    ))
    # Break-even → infinity
    cases.append(make_case(
        "crit_aum_at_breakeven_inf",
        {"pi_pac": 5_000_000.0, "tau_be_h": 168.0, "tau_h": 168.0,
         "theta_impact": 0.5, "leverage": 2, "m_pos": 0.02},
        {"result": "inf"}, 0.0,
        "τ = τ^BE → critical AUM = ∞"
    ))

    write_fixtures(f"{output_dir}/critical_aum.json", cases)
    return len(cases)


# --- §7. Bernstein leverage bound -----------------------------------------

def gen_bernstein_leverage(output_dir):
    cases = []
    # τ → 0: L^R → 1/MMR
    cases.append(make_case(
        "bernstein_tau_zero",
        {"mmr": 0.05, "delta_per_h": 0.01, "sigma_per_h": 0.005,
         "tau_h": 0.0, "epsilon": 0.001},
        {"result": bernstein_leverage_bound(0.05, 0.01, 0.005, 0.0, 0.001)},
        0.0, "τ=0 → L ≈ 1/MMR (integer floor)"
    ))
    # Typical: 1-day, 1-week, 1-month holds
    for tau_h in [24.0, 168.0, 720.0]:
        for eps in [1e-2, 1e-3, 1e-4]:
            cases.append(make_case(
                f"bernstein_tau_{tau_h}_eps_{eps}",
                {"mmr": 0.05, "delta_per_h": 0.01, "sigma_per_h": 0.005,
                 "tau_h": tau_h, "epsilon": eps},
                {"result": bernstein_leverage_bound(
                    0.05, 0.01, 0.005, tau_h, eps)},
                0.0, f"τ={tau_h}h, ε={eps}"
            ))
    # High variance → lower L
    cases.append(make_case(
        "bernstein_high_variance",
        {"mmr": 0.05, "delta_per_h": 0.05, "sigma_per_h": 0.02,
         "tau_h": 168.0, "epsilon": 1e-3},
        {"result": bernstein_leverage_bound(0.05, 0.05, 0.02, 168.0, 1e-3)},
        0.0, "High vol → tight L"
    ))

    write_fixtures(f"{output_dir}/bernstein_leverage.json", cases)
    return len(cases)


# --- §8. MFG competitor count ---------------------------------------------

def gen_mfg_competitor(output_dir):
    cases = []
    # Low signal → K = 0
    cases.append(make_case(
        "mfg_zero_signal",
        {"pi_pac": 5_000_000.0, "d_eff": 0.001,
         "theta_impact": 0.5, "c_op_marginal": 50_000.0},
        {"result": mfg_competitor_count(5_000_000.0, 0.001, 0.5, 50_000.0)},
        1e-9, "Weak signal → no competitors"
    ))
    # Typical
    for mu_val in [0.05, 0.10, 0.15, 0.20]:
        cases.append(make_case(
            f"mfg_typical_mu_{mu_val}",
            {"pi_pac": 5_000_000.0, "d_eff": mu_val,
             "theta_impact": 0.5, "c_op_marginal": 50_000.0},
            {"result": mfg_competitor_count(5_000_000.0, mu_val, 0.5, 50_000.0)},
            1e-9, f"μ = {mu_val}"
        ))
    # Low C_op → more competitors
    cases.append(make_case(
        "mfg_low_cop",
        {"pi_pac": 5_000_000.0, "d_eff": 0.10,
         "theta_impact": 0.5, "c_op_marginal": 10_000.0},
        {"result": mfg_competitor_count(5_000_000.0, 0.10, 0.5, 10_000.0)},
        1e-9, "Cheap ops → crowded"
    ))

    write_fixtures(f"{output_dir}/mfg_competitor.json", cases)
    return len(cases)


# --- §9. Dol sustainable flow + capacity ceiling --------------------------

def gen_dol_sustainable_flow(output_dir):
    cases = []
    cases.append(make_case(
        "dol_flow_positive_edge",
        {"c_op_marginal": 50_000.0, "c_op_dol": 20_000.0},
        {"result": 30_000.0}, 1e-12,
        "Dol has $30k/yr/pair edge"
    ))
    cases.append(make_case(
        "dol_flow_no_edge",
        {"c_op_marginal": 20_000.0, "c_op_dol": 20_000.0},
        {"result": None}, 0.0,
        "No edge → None (error)"
    ))
    cases.append(make_case(
        "dol_flow_negative_edge",
        {"c_op_marginal": 10_000.0, "c_op_dol": 20_000.0},
        {"result": None}, 0.0,
        "Dol more expensive → None"
    ))

    write_fixtures(f"{output_dir}/dol_sustainable_flow.json", cases)
    return len(cases)


def gen_capacity_ceiling(output_dir):
    cases = []
    cases.append(make_case(
        "capacity_typical",
        {"n_active_pairs": 40, "delta_c_op": 30_000.0,
         "r_floor": 0.08, "alpha_min": 0.50, "r_idle": 0.044},
        {"result": capacity_ceiling(40, 30_000.0, 0.08, 0.50, 0.044)},
        1e-6, "N=40, ΔC_op=30k, R_floor=8%, α_min=0.5, r_idle=4.4%"
    ))
    cases.append(make_case(
        "capacity_scale_with_pairs",
        {"n_active_pairs": 100, "delta_c_op": 30_000.0,
         "r_floor": 0.08, "alpha_min": 0.50, "r_idle": 0.044},
        {"result": capacity_ceiling(100, 30_000.0, 0.08, 0.50, 0.044)},
        1e-6, "Scales linear with N"
    ))
    # Infeasible: floor too low relative to idle
    cases.append(make_case(
        "capacity_infeasible_floor",
        {"n_active_pairs": 40, "delta_c_op": 30_000.0,
         "r_floor": 0.02, "alpha_min": 0.50, "r_idle": 0.044},
        {"result": None}, 0.0,
        "R_floor < α·r_idle → None"
    ))

    write_fixtures(f"{output_dir}/capacity_ceiling.json", cases)
    return len(cases)


# --- §10. Cap routing + mandate floor -------------------------------------

def gen_cap_routing(output_dir):
    cases = []
    cust_max, buf_max = 0.08, 0.05
    cut_c, cut_b, cut_r = 0.65, 0.25, 0.10

    test_grosses = [
        0.0, 0.04, 0.08, 0.12308,   # customer cap boundary
        0.20, 0.25, 0.30, 0.50,
    ]
    for r in test_grosses:
        c, b, res = cap_routing_reference(r, cut_c, cut_b, cut_r, cust_max, buf_max)
        cases.append(make_case(
            f"cap_routing_gross_{r}",
            {"vault_gross": r,
             "cut_customer": cut_c, "cut_buffer": cut_b, "cut_reserve": cut_r,
             "cust_max": cust_max, "buf_max": buf_max},
            {"customer": c, "buffer": b, "reserve": res},
            1e-12, f"Gross {r} routing"
        ))

    # Conservation test
    r = 0.20
    c, b, res = cap_routing_reference(r, cut_c, cut_b, cut_r, cust_max, buf_max)
    cases.append(make_case(
        "cap_routing_conservation",
        {"vault_gross": r,
         "cut_customer": cut_c, "cut_buffer": cut_b, "cut_reserve": cut_r,
         "cust_max": cust_max, "buf_max": buf_max},
        {"customer": c, "buffer": b, "reserve": res,
         "sum": c + b + res, "sum_equals_gross": abs(c + b + res - r) < 1e-12},
        1e-12, "Sum = R invariant"
    ))

    write_fixtures(f"{output_dir}/cap_routing.json", cases)
    return len(cases)


def gen_mandate_floor(output_dir):
    cases = []
    cases.append(make_case(
        "mandate_floor_standard",
        {"cust_min": 0.05, "cut_customer": 0.65,
         "buf_min": 0.02, "cut_buffer": 0.25},
        {"result": mandate_floor_reference(0.05, 0.65, 0.02, 0.25)},
        1e-12, "Standard 65/25/10, max(5/.65, 2/.25)"
    ))
    cases.append(make_case(
        "mandate_floor_equal_bind",
        {"cust_min": 0.052, "cut_customer": 0.65,
         "buf_min": 0.02, "cut_buffer": 0.25},
        {"result": mandate_floor_reference(0.052, 0.65, 0.02, 0.25)},
        1e-12, "Both binding simultaneously"
    ))

    write_fixtures(f"{output_dir}/mandate_floor.json", cases)
    return len(cases)


# --- §11. Slippage + round trip cost --------------------------------------

def gen_slippage(output_dir):
    cases = []
    # Floor (small n)
    cases.append(make_case(
        "slip_floor_small_n",
        {"notional_usd": 100.0, "oi_usd": 5_000_000.0, "vol_24h_usd": 10_000_000.0},
        {"result": cost_slippage(100.0, 5_000_000.0, 10_000_000.0)},
        1e-12, "Small order hits slippage floor"
    ))
    # Ceiling (huge n on thin market)
    cases.append(make_case(
        "slip_ceiling_thin",
        {"notional_usd": 10_000_000.0, "oi_usd": 100_000.0, "vol_24h_usd": 10_000.0},
        {"result": cost_slippage(10_000_000.0, 100_000.0, 10_000.0)},
        1e-12, "Huge order on thin market hits ceiling"
    ))
    # Typical
    for n in [1_000.0, 10_000.0, 100_000.0, 1_000_000.0]:
        cases.append(make_case(
            f"slip_typical_n_{n}",
            {"notional_usd": n, "oi_usd": 5_000_000.0, "vol_24h_usd": 10_000_000.0},
            {"result": cost_slippage(n, 5_000_000.0, 10_000_000.0)},
            1e-12, f"n = ${n}"
        ))
    # Zero notional
    cases.append(make_case(
        "slip_zero_n",
        {"notional_usd": 0.0, "oi_usd": 5_000_000.0, "vol_24h_usd": 10_000_000.0},
        {"result": 0.0}, 0.0,
        "n = 0 → slippage 0"
    ))

    write_fixtures(f"{output_dir}/slippage.json", cases)
    return len(cases)


def gen_round_trip_cost(output_dir):
    cases = []
    # Model A baseline
    typical = [
        (0.0004, 0.0005, 0.0001, 0.0001, 0.0),       # cheap
        (0.0004, 0.0005, 0.0005, 0.0005, 0.001),    # typical + bridge
        (0.0010, 0.0010, 0.001, 0.001, 0.0015),     # stress
    ]
    for phi_t_p, phi_t_c, slip_p, slip_c, bridge in typical:
        cases.append(make_case(
            f"rt_model_a_phi_{phi_t_p}_{phi_t_c}_slip_{slip_p}_{slip_c}_br_{bridge}",
            {"phi_t_p": phi_t_p, "phi_t_c": phi_t_c,
             "slip_p": slip_p, "slip_c": slip_c, "bridge_rt": bridge},
            {"result": round_trip_cost_model_a(phi_t_p, phi_t_c, slip_p, slip_c, bridge)},
            1e-12, "Model A: both legs taker"
        ))

    # Model C hybrid
    for phi_m_p, phi_t_p, phi_t_c, slip_p, slip_c, bridge, legw, sig in [
        (0.00015, 0.00040, 0.00050, 0.0001, 0.0001, 0.0, 0.5, 0.05),
        (0.00010, 0.00040, 0.00050, 0.0005, 0.0005, 0.001, 1.0, 0.05),
        (0.00000, 0.00040, 0.00050, 0.001, 0.001, 0.0015, 0.5, 0.10),
    ]:
        cases.append(make_case(
            f"rt_model_c_phi_m_{phi_m_p}_legw_{legw}",
            {"phi_m_p": phi_m_p, "phi_t_p": phi_t_p, "phi_t_c": phi_t_c,
             "slip_p": slip_p, "slip_c": slip_c, "bridge_rt": bridge,
             "legging_window_seconds": legw,
             "sigma_price_per_sqrt_day": sig},
            {"result": round_trip_cost_model_c(
                phi_m_p, phi_t_p, phi_t_c, slip_p, slip_c, bridge, legw, sig)},
            1e-12, "Model C: maker + taker hybrid"
        ))

    write_fixtures(f"{output_dir}/round_trip_cost.json", cases)
    return len(cases)


# --- §12. OU MLE recovery -------------------------------------------------

def gen_fit_ou(output_dir):
    cases = []
    # Known parameters, synthetic sample
    known_params = [
        (0.0001, 0.10, 0.0001, 720, 42),
        (0.00005, 0.05, 0.00008, 1440, 7),
        (-0.00003, 0.15, 0.00010, 2000, 99),
    ]
    for mu_h, theta_h, sigma_h, T, seed in known_params:
        sample = _generate_ou_sample(mu_h, theta_h, sigma_h, T=T, seed=seed)
        fit = fit_ou(sample, dt=1.0)
        if fit is None:
            continue
        cases.append(make_case(
            f"fit_ou_mu_{mu_h}_theta_{theta_h}_T_{T}_seed_{seed}",
            {"sample": sample, "dt": 1.0},
            {
                "mu": fit.mu,
                "theta": fit.theta,
                "sigma": fit.sigma,
                "half_life_h": fit.half_life_h,
                "t_statistic": fit.t_statistic,
            },
            1e-9, "OU MLE on synthetic sample"
        ))

    # fit_drift for drift regime
    drift_params = [(0.0002, 0.00005, 720, 10)]
    for mu_h, sigma_h, T, seed in drift_params:
        rng = random.Random(seed)
        sample = [mu_h + rng.gauss(0, sigma_h) for _ in range(T)]
        fit = fit_drift(sample, dt=1.0)
        if fit is None:
            continue
        cases.append(make_case(
            f"fit_drift_mu_{mu_h}_seed_{seed}",
            {"sample": sample, "dt": 1.0},
            {
                "mu": fit.mu,
                "theta": fit.theta,
                "sigma": fit.sigma,
                "t_statistic": fit.t_statistic,
            },
            1e-9, "Drift-mode fit"
        ))

    write_fixtures(f"{output_dir}/fit_ou.json", cases)
    return len(cases)


# --- §13. ADF test --------------------------------------------------------

def gen_adf(output_dir):
    cases = []
    # Random walk (should NOT reject)
    rng = random.Random(42)
    rw = [0.0]
    for _ in range(500):
        rw.append(rw[-1] + rng.gauss(0, 0.0001))
    adf_rw = adf_test(rw, with_constant=True)
    if adf_rw is not None:
        cases.append(make_case(
            "adf_random_walk",
            {"sample_length": len(rw), "sample_head": rw[:10]},
            {
                "statistic": adf_rw.statistic,
                "rejects_unit_root": adf_rw.rejects_unit_root,
            },
            1e-9, "RW should NOT reject unit root. Full sample in fixture."
        ))
        # Store full sample in a separate field for Rust to reproduce
        cases[-1]["input"]["full_sample"] = rw

    # Strong OU (should reject)
    strong_ou = _generate_ou_sample(0.0001, 0.10, 0.00008, T=720, seed=7)
    adf_ou = adf_test(strong_ou, with_constant=True)
    if adf_ou is not None:
        cases.append(make_case(
            "adf_strong_ou",
            {"sample_length": len(strong_ou), "full_sample": strong_ou},
            {
                "statistic": adf_ou.statistic,
                "rejects_unit_root": adf_ou.rejects_unit_root,
            },
            1e-9, "Strong OU should reject unit root"
        ))

    write_fixtures(f"{output_dir}/adf.json", cases)
    return len(cases)


# --- §14. CVaR drawdown stop ---------------------------------------------

def gen_cvar(output_dir):
    cases = []
    # Empty → bootstrap fallback
    cases.append(make_case(
        "cvar_empty",
        {"basis_history": [], "q": 0.01, "safety_multiplier": 2.0},
        {"result": cvar_drawdown_stop([], q=0.01, safety_multiplier=2.0)},
        1e-12, "Empty → bootstrap"
    ))
    # Normal distribution
    rng = random.Random(123)
    samples = [rng.gauss(0, 0.005) for _ in range(500)]
    cases.append(make_case(
        "cvar_gaussian",
        {"basis_history": samples, "q": 0.01, "safety_multiplier": 2.0},
        {"result": cvar_drawdown_stop(samples, q=0.01, safety_multiplier=2.0)},
        1e-9, "Gaussian basis history"
    ))
    # Fat-tailed
    fat = [rng.gauss(0, 0.005) for _ in range(500)]
    fat[100] = 0.05
    fat[200] = 0.08
    fat[300] = 0.10
    cases.append(make_case(
        "cvar_fat_tailed",
        {"basis_history": fat, "q": 0.01, "safety_multiplier": 2.0},
        {"result": cvar_drawdown_stop(fat, q=0.01, safety_multiplier=2.0)},
        1e-9, "Fat-tailed (injected outliers)"
    ))

    write_fixtures(f"{output_dir}/cvar.json", cases)
    return len(cases)


# --- §15. Hurst DFA ------------------------------------------------------

def gen_hurst(output_dir):
    cases = []
    # Random walk → H ≈ 1
    rng = random.Random(42)
    rw = [0.0]
    for _ in range(1000):
        rw.append(rw[-1] + rng.gauss(0, 0.001))
    h_rw = hurst_dfa(rw)
    cases.append(make_case(
        "hurst_random_walk",
        {"full_sample": rw},
        {"result": h_rw},
        1e-9, "Random walk → H ≈ 1"
    ))
    # White noise → H ≈ 0.5
    white = [rng.gauss(0, 0.001) for _ in range(1000)]
    h_wh = hurst_dfa(white)
    cases.append(make_case(
        "hurst_white_noise",
        {"full_sample": white},
        {"result": h_wh},
        1e-9, "White noise → H ≈ 0.5"
    ))
    # Strong OU → H < 0.5 (anti-persistent after first difference)
    ou = _generate_ou_sample(0.0, 0.1, 0.0001, T=1000, seed=77)
    h_ou = hurst_dfa(ou)
    cases.append(make_case(
        "hurst_ou_sample",
        {"full_sample": ou},
        {"result": h_ou},
        1e-9, "OU sample, measured Hurst"
    ))

    write_fixtures(f"{output_dir}/hurst.json", cases)
    return len(cases)


# --- §16. Expected residual income (OU conditional integral) --------------

def gen_expected_residual_income(output_dir):
    cases = []
    # theta = 0 → direction·mu·hold_h (drift regime)
    cases.append(make_case(
        "eri_drift_regime",
        {"s_now": 0.001, "mu": 0.0001, "theta": 0.0,
         "hold_h": 168.0, "direction": 1},
        {"result": expected_residual_income(0.001, 0.0001, 0.0, 168.0, 1)},
        1e-12, "θ=0 → μ·hold"
    ))
    # theta > 0 typical
    cases.append(make_case(
        "eri_ou_regime",
        {"s_now": 0.0002, "mu": 0.0001, "theta": 0.1,
         "hold_h": 20.0, "direction": 1},
        {"result": expected_residual_income(0.0002, 0.0001, 0.1, 20.0, 1)},
        1e-12, "OU integral with decay"
    ))
    # Negative direction
    cases.append(make_case(
        "eri_negative_direction",
        {"s_now": -0.001, "mu": -0.0005, "theta": 0.05,
         "hold_h": 100.0, "direction": -1},
        {"result": expected_residual_income(-0.001, -0.0005, 0.05, 100.0, -1)},
        1e-12, "Negative direction (short signal)"
    ))

    write_fixtures(f"{output_dir}/expected_residual_income.json", cases)
    return len(cases)


# --- §17. Lifecycle annualized return (legacy v3.5.1 fixed bug) ----------

def gen_lifecycle(output_dir):
    cases = []
    # Typical operating point
    typical = [
        (0.10, 336.0, 0.002, 3, 0.5, 0.044),
        (0.15, 168.0, 0.0015, 2, 0.5, 0.044),
        (0.08, 720.0, 0.002, 3, 0.5, 0.05),
    ]
    for spread, hold_h, c, L, alpha, r_idle in typical:
        result = lifecycle_annualized_return(
            per_pair_spread_apy=spread,
            commitment_hold_h=hold_h,
            c_round_trip=c,
            leverage=L,
            alpha=alpha,
            r_idle=r_idle,
        )
        cases.append(make_case(
            f"lifecycle_spread_{spread}_hold_{hold_h}_L_{L}",
            {"per_pair_spread_apy": spread, "commitment_hold_h": hold_h,
             "c_round_trip": c, "leverage": L,
             "alpha": alpha, "r_idle": r_idle},
            {
                "vault_gross": result["vault_gross"],
                "customer": result["customer"],
                "buffer": result["buffer"],
                "reserve": result["reserve"],
                "rotations_per_year": result["rotations_per_year"],
                "net_on_margin": result["net_on_margin"],
            },
            1e-12, "Python lifecycle reference"
        ))

    write_fixtures(f"{output_dir}/lifecycle.json", cases)
    return len(cases)


# --- §18. End-to-end dry run fixture --------------------------------------

def gen_dry_run_end_to_end(output_dir):
    """NOT a full fixture — just a stub pointer.

    Full E2E fixture requires building LiveInputs from historical data
    and running compute_rigorous_state. This is a large fixture and should
    be generated separately by `scripts/generate_e2e_fixture.py` with
    explicit historical SQLite input.

    Here we write a structural stub so Rust knows the expected shape.
    """
    cases = [{
        "name": "dry_run_e2e_stub",
        "input": {
            "note": "Build LiveInputs from historical_cross_venue.sqlite + Mandate defaults",
            "historical_db": "data/historical_cross_venue.sqlite",
            "mandate": "default",
        },
        "expected": {
            "note": "Run compute_rigorous_state and match all numeric fields to 6 decimal places.",
            "fields_to_match": [
                "n_universe_scanned",
                "n_passing_filters",
                "leverage",
                "chance_constrained.feasible",
                "chance_constrained.idle_alpha",
                "chance_constrained.portfolio_mean_apy",
                "chance_constrained.vault_5pct_apy",
                "chance_constrained.vault_1pct_apy",
            ],
        },
        "tolerance": 1e-6,
        "notes": "Full E2E requires a separate generation script.",
    }]

    write_fixtures(f"{output_dir}/dry_run_end_to_end.json", cases)
    return len(cases)


# =============================================================================
# Main
# =============================================================================

ALL_SECTIONS = {
    "phi": gen_phi,
    "ou": gen_ou_time_averaged_spread,
    "impact": gen_effective_spread_with_impact,
    "break_even": gen_break_even_hold,
    "optimal_notional": gen_optimal_notional,
    "optimal_contribution": gen_optimal_trading_contribution,
    "critical_aum": gen_critical_aum,
    "bernstein": gen_bernstein_leverage,
    "mfg": gen_mfg_competitor,
    "dol_flow": gen_dol_sustainable_flow,
    "capacity": gen_capacity_ceiling,
    "cap_routing": gen_cap_routing,
    "mandate_floor": gen_mandate_floor,
    "slippage": gen_slippage,
    "round_trip": gen_round_trip_cost,
    "fit_ou": gen_fit_ou,
    "adf": gen_adf,
    "cvar": gen_cvar,
    "hurst": gen_hurst,
    "expected_residual": gen_expected_residual_income,
    "lifecycle": gen_lifecycle,
    "dry_run": gen_dry_run_end_to_end,
}


def main():
    parser = argparse.ArgumentParser(
        description="Generate JSON fixtures for Rust parity testing"
    )
    parser.add_argument(
        "--output", "-o",
        default="rust_fixtures",
        help="Output directory (default: rust_fixtures)",
    )
    parser.add_argument(
        "--section", "-s",
        default="all",
        help="Comma-separated sections or 'all'. "
             f"Available: {','.join(ALL_SECTIONS.keys())}",
    )
    args = parser.parse_args()

    sections = list(ALL_SECTIONS.keys()) if args.section == "all" \
        else args.section.split(",")

    print(f"Generating fixtures in: {args.output}")
    print(f"Sections: {sections}")
    print()

    total = 0
    for name in sections:
        if name not in ALL_SECTIONS:
            print(f"  [SKIP] unknown section: {name}")
            continue
        try:
            n = ALL_SECTIONS[name](args.output)
            print(f"  [OK]   {name:22s} {n:>4} cases")
            total += n
        except Exception as e:
            print(f"  [FAIL] {name:22s} {e}")
            raise

    print()
    print(f"Total: {total} fixture cases across {len(sections)} sections")
    print(f"Location: {os.path.abspath(args.output)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
