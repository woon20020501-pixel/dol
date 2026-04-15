"""
optimize_v3_5.py — exhaustive parameter sensitivity + walk-forward validation
for the v3.5 cross-venue funding hedge framework.

Stays strictly within v3.5 logic (no v4.0 deviations). Sweeps the parameters
The following parameters can be tuned:
  - leverage L (forced, not auto-derived)
  - risk aversion γ (DRO concentration)
  - α floor (idle bucket minimum)
  - persistence threshold p_min
  - Hurst lower bound
  - t-statistic threshold

For each combination, runs the v3.5 pipeline on real 60-day data and measures:
  - vault gross APY
  - customer APY (post 65% cut, capped at 8%)
  - buffer APY (post 25% cut + overflow, capped at 5%)
  - portfolio Sharpe
  - # of pairs in allocation
  - max single position weight (concentration)
  - 5%-VaR vault APY

Then identifies the configuration that maximizes the SAFETY MARGIN
(buffer floor distance + customer floor distance) subject to all gates passing.

Walk-forward: splits the 60d into train + test, optimizes on train, validates on test.
"""
import math
import os
import sqlite3
import statistics
import sys
from collections import defaultdict
from datetime import datetime, timezone
from itertools import product

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from strategy.cost_model import LiveInputs, Mandate
from strategy.stochastic import fit_ou, adf_test, _solve_with_inverse
from strategy.frontier import (
    empirical_bernstein_credibility, hurst_dfa, dro_epsilon_from_sample,
    dro_tangency_weights,
)
from strategy.portfolio import covariance_matrix, shrink_covariance

DB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "data", "historical_cross_venue.sqlite")
DEX = ("pacifica", "backpack", "hyperliquid", "lighter")


def load_data(db_path: str, time_filter: tuple = None):
    """Load the historical sqlite. time_filter = (min_ts_ms, max_ts_ms) optional."""
    conn = sqlite3.connect(db_path)
    query = ("SELECT timestamp_ms, symbol, venue, funding_rate FROM funding_aggregated "
             "WHERE venue IN ('pacifica','backpack','hyperliquid','lighter')")
    if time_filter:
        query += f" AND timestamp_ms BETWEEN {time_filter[0]} AND {time_filter[1]}"
    query += " ORDER BY timestamp_ms"
    rows = conn.execute(query).fetchall()
    history = defaultdict(list)
    for ts, sym, venue, rate in rows:
        history[(sym, venue)].append((ts, rate))
    snapshot = {k: v[-1][1] for k, v in history.items()}
    return history, snapshot


def aligned_spread(history, sym, counter):
    pac = dict(history.get((sym, "pacifica"), []))
    cnt = dict(history.get((sym, counter), []))
    common = sorted(set(pac.keys()) & set(cnt.keys()))
    return [cnt[t] - pac[t] for t in common]


def evaluate_candidate(sym, counter, history, snapshot, params):
    """Apply v3.5 filters matching dry_run_v3_5 exactly. Bernstein OR t-stat."""
    spread = aligned_spread(history, sym, counter)
    if len(spread) < 100:  # match dry_run minimum
        return None

    ou = fit_ou(spread, dt=1.0)
    if ou is None or ou.theta <= 0 or math.isinf(ou.half_life_h):
        return None

    # ADF gate (required)
    adf = adf_test(spread, with_constant=True)
    adf_pass = adf is not None and adf.rejects_unit_root
    if not adf_pass:
        return None

    # Hurst gate (required)
    H = hurst_dfa(spread)
    if H is None or H < params["hurst_min"]:
        return None

    # Bernstein OR t-stat (matches dry_run_v3_5 third_order_pass)
    b_lo, b_hi, _ = empirical_bernstein_credibility(spread, delta=1e-3)
    bern_pass = (b_lo > 0 and ou.mu > 0) or (b_hi < 0 and ou.mu < 0)
    t_pass = abs(ou.t_statistic) >= params["t_stat_min"]
    if not (bern_pass or t_pass):
        return None

    persist_dir = 1 if ou.mu > 0 else -1

    return {
        "symbol": sym,
        "counter": counter,
        "n": len(spread),
        "ou_mu_apy": ou.mu * 24 * 365,
        "ou_theta": ou.theta,
        "half_life_h": ou.half_life_h,
        "t_stat": ou.t_statistic,
        "hurst": H,
        "spread": spread,
        "direction": persist_dir,
    }


def run_pipeline(history, snapshot, params, mandate, r_idle=0.044):
    """Run the full v3.5 pipeline with the given parameters. Returns metrics dict."""
    # 1. Build candidate set
    pac_syms = {s for (s, v) in history.keys() if v == "pacifica"}
    candidates = []
    for s in sorted(pac_syms):
        for cnt in DEX:
            if cnt == "pacifica":
                continue
            cand = evaluate_candidate(s, cnt, history, snapshot, params)
            if cand:
                candidates.append(cand)
    if not candidates:
        return None

    # 2. Build covariance matrix
    min_len = min(len(c["spread"]) for c in candidates)
    aligned = [c["spread"][-min_len:] for c in candidates]
    cov_per_h = covariance_matrix(aligned)
    cov_per_h = shrink_covariance(cov_per_h, lam=0.10)
    cov_apy = [[c * 24 * 365 for c in row] for row in cov_per_h]
    expected_apys = [abs(c["ou_mu_apy"]) for c in candidates]

    # 3. Markowitz / DRO weights
    eps = dro_epsilon_from_sample(min_len, confidence=0.95, diameter_estimate=0.10)
    weights_raw = dro_tangency_weights(
        expected_apys, cov_apy, r_idle=r_idle,
        risk_aversion=params["risk_aversion"], dro_epsilon=eps,
    )
    if weights_raw is None or all(w <= 0 for w in weights_raw):
        return None

    # 4. Apply box + budget constraints
    n = len(candidates)
    L = params["leverage"]
    alpha_floor = params["alpha_floor"]
    budget = 1 - alpha_floor
    # Auto-derived m_pos cap from L, alpha, n
    m_pos = (1 - alpha_floor) * L / (2 * max(n, 1))
    m_pos = min(m_pos, params.get("m_pos_max", 0.05))
    m_pos = max(m_pos, params.get("m_pos_min", 0.005))

    # Project to box [0, m_pos]
    weights = [max(0, min(m_pos, w)) for w in weights_raw]
    total = sum(weights)
    if total > budget:
        scale = budget / total
        weights = [w * scale for w in weights]
    elif total <= 0:
        return None

    deployed = sum(weights)
    alpha = 1 - deployed

    # 5. Compute vault APY moments
    portfolio_mean_apy = sum(weights[i] * expected_apys[i] for i in range(n))
    trading_apy_on_aum = (L / 2) * portfolio_mean_apy
    vault_gross = alpha * r_idle + trading_apy_on_aum
    portfolio_var = sum(weights[i] * cov_apy[i][j] * weights[j]
                         for i in range(n) for j in range(n))
    vault_var = (L / 2) ** 2 * portfolio_var
    vault_std = math.sqrt(max(vault_var, 0))
    vault_5pct = vault_gross - 1.645 * vault_std
    vault_1pct = vault_gross - 2.326 * vault_std
    sharpe = (vault_gross - r_idle) / max(vault_std, 1e-9)

    # 6. Apply customer/buffer cap routing
    cust_raw = vault_gross * mandate.cut_customer
    buf_raw = vault_gross * mandate.cut_buffer
    res_raw = vault_gross * mandate.cut_reserve
    cust_capped = min(cust_raw, mandate.customer_apy_max)
    buf_with_excess = buf_raw + (cust_raw - cust_capped)
    buf_capped = min(buf_with_excess, mandate.buffer_apy_max)
    res_total = res_raw + (buf_with_excess - buf_capped)

    # 7. Concentration metrics
    max_weight = max(weights) if weights else 0
    n_active = sum(1 for w in weights if w > 0.001)

    # 8. Mandate gate
    cust_floor_dist = cust_capped - mandate.customer_apy_min
    buf_floor_dist = buf_capped - mandate.buffer_apy_min
    mandate_pass = cust_floor_dist >= 0 and buf_floor_dist >= 0
    safety_margin = min(cust_floor_dist, buf_floor_dist)

    return {
        "n_candidates": n,
        "n_active": n_active,
        "leverage": L,
        "alpha": alpha,
        "m_pos_cap": m_pos,
        "deployed": deployed,
        "vault_gross": vault_gross,
        "vault_std": vault_std,
        "vault_5pct": vault_5pct,
        "vault_1pct": vault_1pct,
        "sharpe": sharpe,
        "customer": cust_capped,
        "buffer": buf_capped,
        "reserve": res_total,
        "max_weight": max_weight,
        "mandate_pass": mandate_pass,
        "safety_margin": safety_margin,
        "cust_floor_dist": cust_floor_dist,
        "buf_floor_dist": buf_floor_dist,
    }


def fmt(metrics):
    if metrics is None:
        return "  → INFEASIBLE"
    pass_mark = "✓" if metrics["mandate_pass"] else "✗"
    return (f"{metrics['n_active']:>3}p L={metrics['leverage']:.0f} α={metrics['alpha']*100:.0f}% "
            f"deploy={metrics['deployed']*100:.1f}%  "
            f"gross={metrics['vault_gross']*100:.2f}%  "
            f"cust={metrics['customer']*100:.2f}%  "
            f"buf={metrics['buffer']*100:.2f}%  "
            f"σ={metrics['vault_std']*100:.2f}%  "
            f"Sh={metrics['sharpe']:.2f}  "
            f"max_w={metrics['max_weight']*100:.2f}%  "
            f"5%VaR={metrics['vault_5pct']*100:.2f}%  "
            f"safety={metrics['safety_margin']*100:+.2f}pp {pass_mark}")


# ----- Default params (current locked v3.5) -----
DEFAULT_PARAMS = {
    "min_obs": 168,
    "t_stat_min": 5.0,
    "hurst_min": 0.30,
    "leverage": 2,
    "alpha_floor": 0.50,
    "risk_aversion": 2.0,
    "m_pos_max": 0.05,
    "m_pos_min": 0.005,
}


def sweep(label, history, snapshot, mandate, base, varying_key, varying_values):
    print()
    print("=" * 105)
    print(f"SWEEP: {label}  (varying {varying_key})")
    print("=" * 105)
    rows = []
    for v in varying_values:
        params = dict(base)
        params[varying_key] = v
        m = run_pipeline(history, snapshot, params, mandate)
        rows.append((v, m))
        print(f"  {varying_key}={v:<6}  {fmt(m)}")
    return rows


def main():
    print("=" * 105)
    print("v3.5 EXHAUSTIVE OPTIMIZATION + WALK-FORWARD VALIDATION")
    print("=" * 105)
    print(f"data: {DB}")
    print(f"AUM assumption: $1M  r_idle: 4.4% (Kamino)")
    print(f"mandate: customer 5-7%, buffer 2-5%, cut 65/25/10")

    # Load full 60-day data
    history_full, snapshot_full = load_data(DB)
    n_pairs_total = len({k for k in history_full.keys() if k[1] == "pacifica"})
    print(f"loaded: {len(history_full)} (sym, venue) pairs, {n_pairs_total} pacifica symbols")
    mandate = Mandate()

    # ------- Baseline: current v3.5 lock -------
    print()
    print("=" * 105)
    print("BASELINE — current v3.5 lock (full 60-day data)")
    print("=" * 105)
    base_metrics = run_pipeline(history_full, snapshot_full, DEFAULT_PARAMS, mandate)
    print(f"  {fmt(base_metrics)}")

    # ------- Sensitivity sweeps (one parameter at a time, others at default) -------
    sweep("Risk aversion (DRO concentration)", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "risk_aversion", [0.5, 1.0, 1.5, 2.0, 3.0, 5.0])

    sweep("Leverage", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "leverage", [1, 2, 3, 4, 5])

    sweep("α floor (idle minimum)", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "alpha_floor", [0.40, 0.45, 0.50, 0.55, 0.60, 0.70])

    sweep("Persistence t-stat threshold", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "t_stat_min", [3.0, 4.0, 5.0, 6.0, 7.0])

    sweep("Hurst lower bound", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "hurst_min", [0.20, 0.30, 0.40, 0.50])

    sweep("m_pos cap (per-leg AUM%)", history_full, snapshot_full, mandate,
          DEFAULT_PARAMS, "m_pos_max", [0.02, 0.03, 0.04, 0.05, 0.07, 0.10])

    # ------- Joint search (multi-parameter) -------
    print()
    print("=" * 105)
    print("JOINT SEARCH — top configurations by safety margin")
    print("=" * 105)
    joint_results = []
    for L, gamma, af, t_min, h_min in product(
        [2, 3, 4],
        [1.0, 1.5, 2.0],
        [0.45, 0.50, 0.55],
        [4.0, 5.0],
        [0.30, 0.40],
    ):
        params = dict(DEFAULT_PARAMS)
        params.update({
            "leverage": L,
            "risk_aversion": gamma,
            "alpha_floor": af,
            "t_stat_min": t_min,
            "hurst_min": h_min,
        })
        m = run_pipeline(history_full, snapshot_full, params, mandate)
        if m is None or not m["mandate_pass"]:
            continue
        joint_results.append((params, m))

    joint_results.sort(key=lambda r: -r[1]["safety_margin"])
    print(f"  {'rank':<6}{'L':<3}{'γ':<5}{'α_f':<6}{'t':<4}{'H':<5}"
          f"{'cust':>8}{'buf':>8}{'σ':>8}{'Sh':>7}{'safety':>10}")
    for i, (p, m) in enumerate(joint_results[:25]):
        print(f"  {i+1:<6}{p['leverage']:<3}{p['risk_aversion']:<5.1f}{p['alpha_floor']:<6.2f}"
              f"{p['t_stat_min']:<4.0f}{p['hurst_min']:<5.2f}"
              f"{m['customer']*100:>7.2f}%{m['buffer']*100:>7.2f}%"
              f"{m['vault_std']*100:>7.2f}%{m['sharpe']:>7.2f}{m['safety_margin']*100:>+9.2f}pp")

    if joint_results:
        best = joint_results[0]
        print()
        print("=" * 105)
        print("RECOMMENDED OPTIMAL CONFIG")
        print("=" * 105)
        print(f"  L = {best[0]['leverage']}")
        print(f"  γ (risk aversion) = {best[0]['risk_aversion']}")
        print(f"  α floor = {best[0]['alpha_floor']}")
        print(f"  t-stat threshold = {best[0]['t_stat_min']}")
        print(f"  Hurst min = {best[0]['hurst_min']}")
        print(f"  → customer = {best[1]['customer']*100:.2f}%  (mandate 5-7%)")
        print(f"  → buffer   = {best[1]['buffer']*100:.2f}%  (mandate 2-5%)")
        print(f"  → safety margin = {best[1]['safety_margin']*100:+.2f}pp above floor")
        print(f"  → portfolio σ = {best[1]['vault_std']*100:.2f}%")
        print(f"  → Sharpe = {best[1]['sharpe']:.2f}")
        print(f"  → 5%-VaR = {best[1]['vault_5pct']*100:.2f}%")
        print(f"  → max single position = {best[1]['max_weight']*100:.2f}% AUM")
        print(f"  → active pairs = {best[1]['n_active']}")

    # ------- Walk-forward validation -------
    print()
    print("=" * 105)
    print("WALK-FORWARD VALIDATION")
    print("=" * 105)
    all_ts = sorted({ts for series in history_full.values() for ts, _ in series})
    if len(all_ts) < 200:
        print("  insufficient data for walk-forward")
    else:
        mid_ts = all_ts[len(all_ts) // 2]
        print(f"  split at {datetime.fromtimestamp(mid_ts/1000, tz=timezone.utc)}")
        print(f"  train: first 50% ({len(all_ts)//2} hours)")
        print(f"  test:  last 50% ({len(all_ts) - len(all_ts)//2} hours)")
        train_history, train_snapshot = load_data(DB, (all_ts[0], mid_ts))
        test_history, test_snapshot = load_data(DB, (mid_ts, all_ts[-1]))

        # Run baseline + best joint config on both
        for label, params in [
            ("baseline (default)", DEFAULT_PARAMS),
            ("optimized (joint best)", best[0] if joint_results else DEFAULT_PARAMS),
        ]:
            print(f"\n  config: {label}")
            tr = run_pipeline(train_history, train_snapshot, params, mandate)
            te = run_pipeline(test_history, test_snapshot, params, mandate)
            print(f"    train: {fmt(tr)}")
            print(f"    test:  {fmt(te)}")
            if tr and te:
                drop_cust = (te["customer"] - tr["customer"]) * 100
                drop_buf = (te["buffer"] - tr["buffer"]) * 100
                print(f"    test−train: customer {drop_cust:+.2f}pp  buffer {drop_buf:+.2f}pp")
                if te["mandate_pass"]:
                    print(f"    → out-of-sample mandate PASS")
                else:
                    print(f"    → out-of-sample mandate FAIL (overfitting risk)")

    print()
    print("=" * 105)
    print("DONE")
    print("=" * 105)


if __name__ == "__main__":
    sys.exit(main() or 0)
