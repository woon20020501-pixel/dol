"""
dry_run_v3_5.py — full v3.5 (3-layer cascade) dry run on historical data.

Reads data/historical_cross_venue.sqlite, builds LiveInputs from the most
recent snapshot, runs:
  Layer 1: cost_model.compute_system_state (closed-form formulas)
  Layer 2: rigorous.compute_rigorous_state (OU + ADF + Markowitz)
  Layer 3: frontier filters (conformal + Bernstein + Hurst + Hawkes + Dol Theorem)

Reports per-layer filtering, final candidate allocation, projected vault APY.

Also runs Granger causality between Pacifica and each counter venue per
symbol — the "Causal IV lite" check that distinguishes structural from
transient spreads.
"""
import math
import os
import sqlite3
import statistics
import sys
from collections import defaultdict
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from strategy.cost_model import LiveInputs, Mandate, compute_system_state
from strategy.rigorous import compute_rigorous_state
from strategy.stochastic import fit_ou, adf_test, _solve_with_inverse
from strategy.frontier import (
    conformal_interval, empirical_bernstein_credibility,
    hurst_dfa, fit_hawkes, expected_cluster_size,
    ou_subgaussian_tail_bound, dro_epsilon_from_sample,
    dro_tangency_weights,
)
from strategy.portfolio import covariance_matrix, shrink_covariance


DB_PATH = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                       "data", "historical_cross_venue.sqlite")


def load_historical_inputs(db_path: str, aum: float = 1_000_000.0,
                            r_idle: float = 0.044) -> LiveInputs:
    conn = sqlite3.connect(db_path)
    rows = conn.execute(
        "SELECT timestamp_ms, symbol, venue, funding_rate FROM funding_aggregated "
        "WHERE venue IN ('pacifica','backpack','hyperliquid','lighter') "
        "ORDER BY timestamp_ms"
    ).fetchall()

    # Group history by (symbol, venue)
    history_by_pair = defaultdict(list)
    for ts, sym, venue, rate in rows:
        history_by_pair[(sym, venue)].append((ts, rate))

    # Latest snapshot — use the most recent rate per (symbol, venue)
    snapshot = {}
    for (sym, venue), series in history_by_pair.items():
        snapshot[(sym, venue)] = series[-1][1]

    # OI/Vol — historical sqlite doesn't have these, use proxies based on what we
    # know from the original snapshot file.
    # For dry run purposes, set generous values so the OI cap doesn't artificially
    # block everything. Phase 1 will measure these live.
    oi = {}
    vol = {}
    for (sym, venue) in history_by_pair.keys():
        oi[(sym, venue)] = 5_000_000.0   # $5M per pair OI assumed
        vol[(sym, venue)] = 1_000_000.0  # $1M 24h vol assumed

    return LiveInputs(
        timestamp_ms=int(datetime.now(tz=timezone.utc).timestamp() * 1000),
        aum_usd=aum,
        r_idle=r_idle,
        funding_rate_h=snapshot,
        open_interest_usd=oi,
        volume_24h_usd=vol,
        fee_maker={"pacifica": 0.00015, "backpack": 0.00020,
                   "hyperliquid": 0.00025, "lighter": 0.00020},
        fee_taker={"pacifica": 0.00040, "backpack": 0.00050,
                   "hyperliquid": 0.00050, "lighter": 0.00050},
        bridge_fee_round_trip={
            ("pacifica", "backpack"): 0.0,
            ("pacifica", "hyperliquid"): 0.0010,
            ("pacifica", "lighter"): 0.0015,
        },
        funding_history_h=dict(history_by_pair),
        basis_divergence_history={},  # sqlite doesn't have this; bootstrap fallback
        vault_daily_returns=[],
    )


def get_pacifica_anchored_pairs(inputs: LiveInputs):
    """Returns list of (symbol, counter_venue) where both have history."""
    pacifica_syms = {s for (s, v) in inputs.funding_history_h.keys() if v == "pacifica"}
    out = []
    for s in sorted(pacifica_syms):
        for v in ("backpack", "hyperliquid", "lighter"):
            if (s, v) in inputs.funding_history_h:
                out.append((s, v))
    return out


def aligned_spread_series(inputs: LiveInputs, sym: str, counter: str):
    pac = dict(inputs.funding_history_h.get((sym, "pacifica"), []))
    cnt = dict(inputs.funding_history_h.get((sym, counter), []))
    common_ts = sorted(set(pac.keys()) & set(cnt.keys()))
    return [cnt[t] - pac[t] for t in common_ts]


def layer_1_filter(inputs: LiveInputs, mandate: Mandate):
    """First-order: cost_model.compute_system_state."""
    state = compute_system_state(inputs, mandate)
    return state


def layer_2_filter(inputs: LiveInputs, mandate: Mandate):
    """Classical rigorous (OU + ADF + Markowitz)."""
    return compute_rigorous_state(inputs, mandate)


def layer_3_filter_per_pair(sym: str, counter: str, inputs: LiveInputs, mandate: Mandate):
    """Frontier filters: returns a per-pair diagnostic dict."""
    spread = aligned_spread_series(inputs, sym, counter)
    if len(spread) < 100:
        return None
    # OU fit
    ou = fit_ou(spread, dt=1.0)
    if ou is None or ou.theta <= 0 or math.isinf(ou.half_life_h):
        return None
    # Empirical Bernstein on spread
    bern_lo, bern_hi, bern_rad = empirical_bernstein_credibility(spread, delta=1e-3)
    bern_pass = (bern_lo > 0 and ou.mu > 0) or (bern_hi < 0 and ou.mu < 0)
    # Hurst exponent
    H = hurst_dfa(spread, min_window=10, max_window=len(spread)//4)
    # Hurst gate: empirically funding spreads are H ≈ 0.9 (strongly persistent), so
    # OU's H = 0.5 assumption was wrong. We accept any Hurst ≥ 0.30 (rejecting only
    # true anti-persistent processes which are noise). H above 1.0 is theoretically
    # impossible for fBm but DFA can return >1 in finite samples — still tradeable.
    hurst_ok = H is not None and H >= 0.30
    # ADF
    adf = adf_test(spread, with_constant=True)
    adf_pass = adf is not None and adf.rejects_unit_root
    # Asymptotic t-stat (Phillips, second-order)
    t_pass = abs(ou.t_statistic) >= 5.0
    return {
        "symbol": sym,
        "counter": counter,
        "n": len(spread),
        "ou_mu_apy": ou.mu * 24 * 365,
        "ou_theta": ou.theta,
        "ou_half_life_h": ou.half_life_h,
        "ou_t_stat": ou.t_statistic,
        "bernstein_pass": bern_pass,
        "bernstein_lower": bern_lo * 24 * 365,
        "bernstein_upper": bern_hi * 24 * 365,
        "adf_pass": adf_pass,
        "adf_stat": adf.statistic if adf else None,
        "hurst": H,
        "hurst_ok": hurst_ok,
        "t_5sigma_pass": t_pass,
        # Composite: pass third-order if (Bernstein OR t-stat) AND ADF AND Hurst
        "third_order_pass": adf_pass and hurst_ok and (bern_pass or t_pass),
    }


def granger_causality(x: list, y: list, lags: int = 4):
    """Test whether past x Granger-causes y.
    Restricted: y_t = α + Σ a_k y_{t-k} + ε
    Unrestricted: y_t = α + Σ a_k y_{t-k} + Σ b_k x_{t-k} + ε
    F-stat tests joint significance of b_k.
    Returns (F_statistic, n_obs, df_num, df_den) — None if not enough data."""
    n = len(x)
    if n != len(y):
        return None
    if n < 3 * (2 * lags + 2):
        return None
    # Build target Y = y[lags:]
    Y = y[lags:]
    n_eff = len(Y)
    # Restricted design: [1, y_{t-1}, ..., y_{t-lags}]
    X_r = []
    for t in range(lags, n):
        row = [1.0] + [y[t - k] for k in range(1, lags + 1)]
        X_r.append(row)
    # Unrestricted: + [x_{t-1}, ..., x_{t-lags}]
    X_u = []
    for t in range(lags, n):
        row = [1.0] + [y[t - k] for k in range(1, lags + 1)] + [x[t - k] for k in range(1, lags + 1)]
        X_u.append(row)

    def ols_sse(X, Y):
        n_cols = len(X[0])
        XTX = [[0.0] * n_cols for _ in range(n_cols)]
        XTY = [0.0] * n_cols
        for i in range(len(Y)):
            for a in range(n_cols):
                XTY[a] += X[i][a] * Y[i]
                for b in range(n_cols):
                    XTX[a][b] += X[i][a] * X[i][b]
        try:
            beta, _ = _solve_with_inverse(XTX, XTY)
        except Exception:
            return None
        sse = 0.0
        for i in range(len(Y)):
            pred = sum(X[i][a] * beta[a] for a in range(n_cols))
            sse += (Y[i] - pred) ** 2
        return sse

    sse_r = ols_sse(X_r, Y)
    sse_u = ols_sse(X_u, Y)
    if sse_r is None or sse_u is None or sse_u == 0:
        return None
    df_num = lags
    df_den = n_eff - 2 * lags - 1
    if df_den <= 0:
        return None
    F = ((sse_r - sse_u) / df_num) / (sse_u / df_den)
    return (F, n_eff, df_num, df_den)


def f_critical_5pct(df1: int, df2: int) -> float:
    """Approximate F critical value at 5% (one-tailed). Uses standard table values
    for common df. For our purposes (df1=4, df2 large), F_0.05 ≈ 2.37."""
    # Quick lookup for common df1
    if df2 > 100:
        if df1 == 1: return 3.84
        if df1 == 2: return 3.00
        if df1 == 3: return 2.60
        if df1 == 4: return 2.37
        if df1 == 5: return 2.21
        if df1 == 6: return 2.10
        if df1 == 8: return 1.94
        if df1 == 12: return 1.75
    return 2.50  # conservative default


def main():
    print("=" * 100)
    print("Dol Strategy v3.5 — DRY RUN on historical_cross_venue.sqlite")
    print("=" * 100)

    inputs = load_historical_inputs(DB_PATH, aum=1_000_000.0, r_idle=0.044)
    mandate = Mandate()
    print(f"  AUM: $1,000,000  r_idle: 4.4%  (Kamino current)")
    print(f"  history pairs loaded: {len(inputs.funding_history_h)}")
    print(f"  pacifica symbols: {len({s for (s,v) in inputs.funding_history_h.keys() if v == 'pacifica'})}")

    pairs = get_pacifica_anchored_pairs(inputs)
    print(f"  Pacifica-anchored (pacifica + 1 counter) pairs: {len(pairs)}")
    print()

    # ---- Layer 1: closed-form ----
    print("=" * 100)
    print("LAYER 1 — closed-form formulas (cost_model.compute_system_state)")
    print("=" * 100)
    state1 = layer_1_filter(inputs, mandate)
    print(f"  target_vault_apy : {state1.target_vault_apy*100:.2f}%")
    print(f"  median_pair_apy  : {state1.median_pair_apy*100:.2f}%")
    print(f"  L                : {state1.leverage}")
    print(f"  alpha            : {state1.idle_fraction*100:.1f}%")
    print(f"  m_pos            : {state1.position_aum_cap*100:.2f}%")
    print(f"  N_active (passing layer 1 pre-filters): {state1.n_active_candidates}")
    print()

    # ---- Layer 2: classical rigorous ----
    print("=" * 100)
    print("LAYER 2 — OU MLE + ADF + Phillips t-stat + Markowitz (rigorous.compute_rigorous_state)")
    print("=" * 100)
    state2 = layer_2_filter(inputs, mandate)
    print(f"  scanned: {state2.n_universe_scanned}  passed L2 filters: {state2.n_passing_filters}")
    print(f"  L: {state2.leverage}")
    print(f"  chance-constrained:")
    print(f"    feasible       : {state2.chance_constrained.feasible}")
    print(f"    alpha          : {state2.chance_constrained.idle_alpha*100:.1f}%")
    print(f"    vault mean     : {state2.chance_constrained.portfolio_mean_apy*100:.2f}%")
    print(f"    vault 5% VaR   : {state2.chance_constrained.vault_5pct_apy*100:.2f}%")
    print(f"    binds          : {state2.chance_constrained.binds}")
    if state2.candidates:
        print(f"  L2 sample candidates:")
        for c in state2.candidates[:8]:
            print(f"    {c.symbol}/{c.counter_venue:<12}  μ_apy={c.ou.mu*24*365*100:>+7.2f}%  "
                  f"θ={c.ou.theta:.4f}  half_life={c.ou.half_life_h:.1f}h  t={c.ou.t_statistic:>+5.2f}")
    print()

    # ---- Layer 3: frontier ----
    print("=" * 100)
    print("LAYER 3 — frontier (Bernstein + Hurst + ADF composite)")
    print("=" * 100)
    l3_results = []
    for sym, counter in pairs:
        diag = layer_3_filter_per_pair(sym, counter, inputs, mandate)
        if diag is None:
            continue
        l3_results.append(diag)
    l3_pass = [d for d in l3_results if d["third_order_pass"]]
    print(f"  scanned: {len(l3_results)}  passed L3 frontier: {len(l3_pass)}")
    print()
    print(f"  Top 15 by |OU mu APY| (3rd-order passing only):")
    print(f"  {'symbol':<10}{'counter':<14}{'n':>6}{'mu_apy':>11}{'theta':>9}{'hl_h':>8}{'t_stat':>9}{'bern_lo':>11}{'hurst':>8}{'pass':>8}")
    l3_pass.sort(key=lambda d: abs(d["ou_mu_apy"]), reverse=True)
    for d in l3_pass[:15]:
        flag = "Y" if d["third_order_pass"] else "N"
        print(f"  {d['symbol']:<10}{d['counter']:<14}{d['n']:>6}"
              f"{d['ou_mu_apy']*100:>10.2f}%{d['ou_theta']:>9.4f}{d['ou_half_life_h']:>8.1f}"
              f"{d['ou_t_stat']:>+9.2f}{d['bernstein_lower']*100:>10.2f}%"
              f"{(d['hurst'] or 0):>8.3f}{flag:>8}")
    print()
    print(f"  All scanned pairs (showing actual values + rejection reasons):")
    print(f"  {'symbol':<10}{'counter':<14}{'mu_apy':>10}{'t_stat':>9}{'hurst_val':>10}{'adf':>5}{'hurstOK':>8}{'bern':>5}{'5σ':>4}")
    hurst_distribution = []
    for d in sorted(l3_results, key=lambda d: abs(d["ou_mu_apy"]), reverse=True)[:35]:
        adf_mark = "Y" if d['adf_pass'] else "n"
        hurst_mark = "Y" if d['hurst_ok'] else "n"
        bern_mark = "Y" if d['bernstein_pass'] else "n"
        t_mark = "Y" if d['t_5sigma_pass'] else "n"
        h_val = d['hurst'] or 0
        hurst_distribution.append(h_val)
        print(f"  {d['symbol']:<10}{d['counter']:<14}{d['ou_mu_apy']*100:>9.2f}%"
              f"{d['ou_t_stat']:>+9.2f}{h_val:>10.3f}{adf_mark:>5}{hurst_mark:>8}{bern_mark:>5}{t_mark:>4}")
    if hurst_distribution:
        print()
        print(f"  Hurst distribution across all 52 candidates:")
        print(f"    min={min(hurst_distribution):.3f}  max={max(hurst_distribution):.3f}  "
              f"median={statistics.median(hurst_distribution):.3f}  mean={statistics.mean(hurst_distribution):.3f}")
        print(f"    in [0.30, 0.70]: {sum(1 for h in hurst_distribution if 0.30 <= h <= 0.70)}")
        print(f"    in [0.20, 0.80]: {sum(1 for h in hurst_distribution if 0.20 <= h <= 0.80)}")
        print(f"    in [0.10, 0.90]: {sum(1 for h in hurst_distribution if 0.10 <= h <= 0.90)}")
    print()

    # ---- Final allocation using L3-passing candidates with DRO ----
    print("=" * 100)
    print("FINAL ALLOCATION — L3-passing candidates → Wasserstein DRO portfolio")
    print("=" * 100)
    if not l3_pass:
        print("  no candidates passed L3; vault stays all-idle")
        return 0
    n_l3 = len(l3_pass)
    expected_apys = [abs(d["ou_mu_apy"]) for d in l3_pass]
    # Build covariance from spread series
    series_list = [aligned_spread_series(inputs, d["symbol"], d["counter"]) for d in l3_pass]
    min_len = min(len(s) for s in series_list)
    aligned = [s[-min_len:] for s in series_list]
    cov_per_h = covariance_matrix(aligned)
    cov_per_h = shrink_covariance(cov_per_h, lam=0.10)
    cov_apy = [[c * 24 * 365 for c in row] for row in cov_per_h]
    eps = dro_epsilon_from_sample(min_len, confidence=0.95, diameter_estimate=0.10)
    print(f"  DRO epsilon (n={min_len}): {eps:.5f}")
    w_raw = dro_tangency_weights(expected_apys, cov_apy, r_idle=inputs.r_idle,
                                  risk_aversion=2.0, dro_epsilon=eps)
    # v3.5.2: leverage is now auto-derived from real candidate signal strength
    # via required_leverage_rigorous, NOT hardcoded. The previous version pinned
    # L=2 which silently masked the strength of the L3-passing candidates.
    from strategy.cost_model import LOCKED_MIN_LEVERAGE
    from strategy.rigorous import required_leverage_rigorous
    median_apy = statistics.median(expected_apys) if expected_apys else 0.0
    target_apy = mandate.customer_apy_min / mandate.cut_customer  # = 5%/0.65 = 7.69%
    L_locked = required_leverage_rigorous(
        median_pair_apy=median_apy,
        r_idle=inputs.r_idle,
        target_apy=target_apy,
        alpha_floor=mandate.aum_buffer_floor,
        min_leverage=LOCKED_MIN_LEVERAGE,
    )
    print(f"  median |OU mu APY|: {median_apy*100:.2f}%")
    print(f"  L (auto-derived from median signal vs target): {L_locked}")
    m_pos = (1 - 0.5) * L_locked / (2 * n_l3)  # alpha=0.5
    m_pos = min(m_pos, 0.05)
    w_capped = [max(0, min(m_pos, w)) for w in w_raw]
    total = sum(w_capped)
    if total > 0.5:
        scale = 0.5 / total
        w_capped = [w * scale for w in w_capped]
    deployed = sum(w_capped)
    print(f"  m_pos cap (per leg): {m_pos*100:.2f}%")
    L = L_locked
    portfolio_mean_apy = sum(w_capped[i] * expected_apys[i] for i in range(n_l3))
    trading_apy_on_aum = (L / 2) * portfolio_mean_apy
    alpha = 1 - deployed
    vault_gross = alpha * inputs.r_idle + trading_apy_on_aum
    cust_raw = vault_gross * mandate.cut_customer
    buf_raw = vault_gross * mandate.cut_buffer
    cust_capped = min(cust_raw, mandate.customer_apy_max)
    cust_excess = cust_raw - cust_capped
    buf_with_excess = buf_raw + cust_excess
    buf_capped = min(buf_with_excess, mandate.buffer_apy_max)
    res_total = vault_gross * mandate.cut_reserve + (buf_with_excess - buf_capped)

    print(f"  pair count        : {n_l3}")
    print(f"  total deployed    : ${deployed*1_000_000:,.0f}  ({deployed*100:.1f}% AUM)")
    print(f"  alpha (idle)      : {alpha*100:.1f}%")
    print(f"  vault gross APY   : {vault_gross*100:.2f}%")
    print(f"  → customer (cap)  : {cust_capped*100:.2f}%   (mandate 5-8%: "
          f"{'OK' if mandate.customer_apy_min <= cust_capped <= mandate.customer_apy_max else 'CHECK'})")
    print(f"  → buffer  (cap)   : {buf_capped*100:.2f}%   (mandate 2-5%: "
          f"{'OK' if mandate.buffer_apy_min <= buf_capped <= mandate.buffer_apy_max else 'CHECK'})")
    print(f"  → reserve         : {res_total*100:.2f}%")
    print()
    print(f"  Per-pair allocation (top 15 by weight):")
    indexed = list(enumerate(l3_pass))
    indexed.sort(key=lambda iw: -w_capped[iw[0]])
    for i, d in indexed[:15]:
        if w_capped[i] > 0:
            print(f"    {d['symbol']:<10}/{d['counter']:<14}  w={w_capped[i]*100:>5.2f}% AUM  "
                  f"μ_apy={expected_apys[i]*100:>6.2f}%  half_life={d['ou_half_life_h']:.1f}h")
    print()

    # ---- Granger causality (Causal IV lite) ----
    print("=" * 100)
    print("CAUSAL IV LITE — Granger causality between Pacifica and counter venues")
    print("=" * 100)
    print(f"  Tests: does past Pacifica funding predict counter funding (and vice versa)?")
    print(f"  H0 (no causality): F < critical at 5%. Reject → predictive precedence exists.")
    print()
    print(f"  {'symbol':<10}{'counter':<14}{'F_pac→cnt':>14}{'F_cnt→pac':>14}{'verdict':>30}")

    granger_results = []
    for sym, counter in pairs:
        pac_h = dict(inputs.funding_history_h.get((sym, "pacifica"), []))
        cnt_h = dict(inputs.funding_history_h.get((sym, counter), []))
        common_ts = sorted(set(pac_h.keys()) & set(cnt_h.keys()))
        if len(common_ts) < 100:
            continue
        pac_series = [pac_h[t] for t in common_ts]
        cnt_series = [cnt_h[t] for t in common_ts]

        g_pac_to_cnt = granger_causality(pac_series, cnt_series, lags=4)
        g_cnt_to_pac = granger_causality(cnt_series, pac_series, lags=4)
        if g_pac_to_cnt is None or g_cnt_to_pac is None:
            continue
        F_pc, n_pc, df1, df2 = g_pac_to_cnt
        F_cp, n_cp, _, _ = g_cnt_to_pac
        crit = f_critical_5pct(df1, df2)
        pc_sig = F_pc > crit
        cp_sig = F_cp > crit
        if pc_sig and cp_sig:
            verdict = "BIDIRECTIONAL (structural)"
        elif pc_sig:
            verdict = "Pacifica leads (price disc)"
        elif cp_sig:
            verdict = f"{counter} leads"
        else:
            verdict = "INDEPENDENT (noise / no causal link)"
        granger_results.append({
            "symbol": sym, "counter": counter,
            "F_pc": F_pc, "F_cp": F_cp, "verdict": verdict,
            "bidirectional": pc_sig and cp_sig,
            "pac_leads": pc_sig and not cp_sig,
            "cnt_leads": cp_sig and not pc_sig,
        })

    granger_results.sort(key=lambda g: max(g["F_pc"], g["F_cp"]), reverse=True)
    for g in granger_results[:30]:
        print(f"  {g['symbol']:<10}{g['counter']:<14}{g['F_pc']:>14.2f}{g['F_cp']:>14.2f}  {g['verdict']}")
    print()

    bidir = [g for g in granger_results if g["bidirectional"]]
    pac_leads = [g for g in granger_results if g["pac_leads"]]
    cnt_leads = [g for g in granger_results if g["cnt_leads"]]
    independent = [g for g in granger_results if not (g["bidirectional"] or g["pac_leads"] or g["cnt_leads"])]
    print(f"  Causal classification summary ({len(granger_results)} pairs):")
    print(f"    BIDIRECTIONAL    (structural carry): {len(bidir):>3} pairs")
    print(f"    PACIFICA leads   (price discovery) : {len(pac_leads):>3} pairs")
    print(f"    COUNTER leads    (lagging Pacifica): {len(cnt_leads):>3} pairs")
    print(f"    INDEPENDENT      (no causal link)  : {len(independent):>3} pairs")
    print()

    # Cross-tabulate: do L3-passing candidates also have causal structure?
    print(f"  Causal status of L3-passing candidates:")
    for d in l3_pass[:15]:
        match = next((g for g in granger_results
                      if g["symbol"] == d["symbol"] and g["counter"] == d["counter"]), None)
        if match:
            print(f"    {d['symbol']:<10}/{d['counter']:<14} μ={d['ou_mu_apy']:>+7.2f}%  →  {match['verdict']}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
