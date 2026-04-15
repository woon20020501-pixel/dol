"""
lifecycle_model.py — measure whether the Dol mandate closes once rotation
cost is included.

The expert review (critique B in the second-pass feedback) pointed out that
every APY number the framework quotes is per-snapshot, not per-year. In a
5-7% mandate product, rotation cost compounds to first-order magnitude:
a 15bp round-trip repeated weekly is 7.8% annual cost — larger than the
entire customer target.

This script:

  1. Measures c_round_trip from real 60-day data on the actual L3-passing
     candidates at realistic position sizes.
  2. Measures the current-regime average per-pair spread from the same data
     (= the s̄ that dry_run's FINAL ALLOCATION implicitly uses).
  3. Sweeps scenarios across (commitment_hold, leverage, spread_regime) and
     reports for each: vault gross, customer, buffer, mandate status,
     breakeven spread.
  4. Identifies which configurations survive the "normal state" scenario
     where spread regresses from the current high to the historical median
     for delta-neutral funding capture (5-10% annualized).

Run: .venv/Scripts/python.exe scripts/lifecycle_model.py
"""
import os
import sqlite3
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from strategy.cost_model import (
    LiveInputs, Mandate, lifecycle_annualized_return,
    round_trip_cost_pct, slippage,
)
from strategy.rigorous import build_spread_series


DB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                  "data", "historical_cross_venue.sqlite")


def load_inputs():
    con = sqlite3.connect(DB)
    rows = con.execute(
        "SELECT timestamp_ms, symbol, venue, funding_rate "
        "FROM funding_aggregated ORDER BY timestamp_ms"
    ).fetchall()
    fh = {}
    for ts, s, v, fr in rows:
        fh.setdefault((s, v), []).append((ts, fr))
    return LiveInputs(
        timestamp_ms=rows[-1][0],
        aum_usd=1_000_000.0,
        r_idle=0.044,
        funding_rate_h={k: v[-1][1] for k, v in fh.items()},
        open_interest_usd={k: 5_000_000.0 for k in fh},
        volume_24h_usd={k: 5_000_000.0 for k in fh},
        fee_maker={"pacifica": 0.00015, "backpack": 0.00020,
                   "hyperliquid": 0.00025, "lighter": 0.00020},
        fee_taker={"pacifica": 0.00040, "backpack": 0.00050,
                   "hyperliquid": 0.00050, "lighter": 0.00050},
        bridge_fee_round_trip={("pacifica", "backpack"): 0.0,
                               ("pacifica", "hyperliquid"): 0.0010,
                               ("pacifica", "lighter"): 0.0015},
        funding_history_h=fh,
        basis_divergence_history={},
        vault_daily_returns=[],
    )


def measure_current_universe(inputs: LiveInputs) -> tuple:
    """Return (candidates with positive signed mean, list of (symbol, counter, spread_apy))."""
    pac_syms = sorted({s for (s, v) in inputs.funding_history_h if v == "pacifica"})
    counters = ["backpack", "hyperliquid"]
    candidates = []
    for s in pac_syms:
        for c in counters:
            if (s, c) not in inputs.funding_history_h:
                continue
            _, spread = build_spread_series(s, c, inputs)
            if len(spread) < 168:
                continue
            signed_mean_per_h = sum(spread) / len(spread)
            if abs(signed_mean_per_h) < 1e-7:
                continue
            candidates.append((s, c, abs(signed_mean_per_h) * 24 * 365))
    return candidates


def measure_c_round_trip(inputs: LiveInputs, candidates: list, aum: float) -> dict:
    """Compute the average round-trip cost in bp for realistic position sizes.
    Uses three size tiers: $1k beta, $100k mid, $1M large."""
    sizes = {"beta ($1k)": 1_000.0,
             "mid ($100k)": 100_000.0,
             "large ($1M)": 1_000_000.0}
    results = {}
    for label, notional in sizes.items():
        costs = []
        for sym, counter, _ in candidates[:20]:
            c = round_trip_cost_pct(sym, "pacifica", counter, notional, inputs)
            costs.append(c)
        if costs:
            results[label] = {
                "median": statistics.median(costs),
                "mean": statistics.mean(costs),
                "max": max(costs),
                "min": min(costs),
            }
    return results


def print_lifecycle_row(label: str, res: dict):
    flag_c = "OK" if res["mandate_customer_ok"] else "MISS"
    flag_b = "OK" if res["mandate_buffer_ok"] else "MISS"
    print(
        f"  {label:<32}  "
        f"gross={res['vault_gross']*100:>6.2f}%  "
        f"cust={res['customer']*100:>5.2f}%[{flag_c}]  "
        f"buf={res['buffer']*100:>5.2f}%[{flag_b}]  "
        f"res={res['reserve']*100:>5.2f}%  "
        f"rot={res['rotations_per_year']:>5.1f}/yr  "
        f"cost_marg={res['annual_cost_on_margin']*100:>5.2f}%"
    )


def main():
    print("=" * 100)
    print("LIFECYCLE COST MODEL — does the Dol mandate close after rotation cost?")
    print("=" * 100)

    inputs = load_inputs()
    m = Mandate()

    # ---- Step 1: real universe ----
    print()
    print("STEP 1 — real universe measurement")
    print("-" * 100)
    candidates = measure_current_universe(inputs)
    spreads = [c[2] for c in candidates]
    print(f"  candidates with history+nonzero mean: {len(candidates)}")
    if spreads:
        print(f"  spread APY — min {min(spreads)*100:.2f}%  "
              f"median {statistics.median(spreads)*100:.2f}%  "
              f"mean {statistics.mean(spreads)*100:.2f}%  "
              f"max {max(spreads)*100:.2f}%")
        top5 = sorted(candidates, key=lambda x: x[2], reverse=True)[:5]
        print("  top 5 by spread:")
        for s, c, sp in top5:
            print(f"    {s:<10}/{c:<12}  {sp*100:>6.2f}% APY")

    # Current-regime s̄ (the actual average spread the framework would trade):
    # Use top-N candidates (deployed ones) — the ones that will be in the portfolio.
    # At m_pos cap with 46 pairs, every pair deploys equally, so s̄ ≈ mean of spreads.
    # But in DRO allocation, higher-spread pairs get capped at m_pos, lower pairs scaled down.
    # The effective s̄ is the MEAN of deployed pairs, which in the L=3/50%-deploy case
    # is the mean of the top (1-α)/m_pos ≈ 30-46 pairs by Sharpe.
    current_s_avg_top_46 = statistics.mean(sorted(spreads, reverse=True)[:46]) if len(spreads) >= 46 else statistics.mean(spreads)
    current_s_avg_all = statistics.mean(spreads) if spreads else 0.0
    current_s_median = statistics.median(spreads) if spreads else 0.0
    print(f"  implied s̄ for framework (top-46 mean, current high-funding regime): "
          f"{current_s_avg_top_46*100:.2f}%")

    # ---- Step 2: realistic c_round_trip ----
    print()
    print("STEP 2 — round-trip cost on realistic position sizes")
    print("-" * 100)
    cost_table = measure_c_round_trip(inputs, candidates, inputs.aum_usd)
    for label, stats in cost_table.items():
        print(f"  {label:<14}  "
              f"min {stats['min']*10000:>5.1f}bp  "
              f"median {stats['median']*10000:>5.1f}bp  "
              f"mean {stats['mean']*10000:>5.1f}bp  "
              f"max {stats['max']*10000:>5.1f}bp")
    c_mid = cost_table.get("mid ($100k)", {}).get("median", 0.0015)
    c_large = cost_table.get("large ($1M)", {}).get("median", 0.005)
    print(f"  — using c_mid = {c_mid*10000:.1f}bp (beta-AUM target) "
          f"and c_large = {c_large*10000:.1f}bp (stress/large-AUM)")

    # ---- Step 3: scenario sweep ----
    print()
    print("STEP 3 — lifecycle sweep across (spread regime, commitment hold, leverage)")
    print("-" * 100)

    # Three spread regimes
    regimes = {
        "current_hi (top-46 mean)": current_s_avg_top_46,
        "moderate (10% — funding norm high)": 0.10,
        "normal  ( 7% — funding norm mid )": 0.07,
        "conservative (5% — funding norm low)": 0.05,
    }
    # Three commitment-hold windows
    holds = [168.0, 336.0, 720.0]  # 1w, 2w, 1m
    hold_labels = {168.0: "1-week", 336.0: "2-week", 720.0: "1-month"}
    # Two leverages
    leverages = [2, 3]
    # Idle & r_idle
    alpha = m.aum_buffer_floor   # 0.50
    r_idle = inputs.r_idle

    # Use mid-tier cost for the table
    c = c_mid

    for regime_label, s_avg in regimes.items():
        print()
        print(f"SCENARIO: spread regime = {regime_label}  (s̄ = {s_avg*100:.2f}%)")
        print(f"{'':34}  {'vault':>8}  {'customer':>9}  {'buffer':>10}  "
              f"{'reserve':>8}  {'rotations':>11}  {'rot_cost':>9}")
        for L in leverages:
            for h in holds:
                res = lifecycle_annualized_return(
                    per_pair_spread_apy=s_avg,
                    commitment_hold_h=h,
                    c_round_trip=c,
                    leverage=L,
                    alpha=alpha,
                    r_idle=r_idle,
                )
                label = f"L={L}, hold={hold_labels[h]}"
                print_lifecycle_row(label, res)
            print()

    # ---- Step 4: breakeven spread table (the operator-actionable view) ----
    print()
    print("STEP 4 — BREAKEVEN SPREAD (s̄ required for customer = 5% mandate floor)")
    print("-" * 100)
    print(f"  Using c = {c*10000:.1f}bp round-trip, α = {alpha:.2f}, r_idle = {r_idle*100:.1f}%")
    print()
    print(f"  {'config':<22}  {'breakeven s̄':>14}  {'current s̄':>12}  {'slack':>10}")
    for L in leverages:
        for h in holds:
            res = lifecycle_annualized_return(
                per_pair_spread_apy=current_s_avg_top_46,
                commitment_hold_h=h,
                c_round_trip=c,
                leverage=L,
                alpha=alpha,
                r_idle=r_idle,
            )
            bk = res["breakeven_spread_apy"]
            slack = current_s_avg_top_46 - bk
            label = f"L={L}, hold={hold_labels[h]}"
            slack_mark = "+" if slack > 0 else "FAIL"
            print(f"  {label:<22}  {bk*100:>13.2f}%  "
                  f"{current_s_avg_top_46*100:>11.2f}%  "
                  f"{slack*100:>+9.2f}pp  {slack_mark}")

    # ---- Step 5: stress — what if the funding regime regresses to historical norm? ----
    print()
    print("STEP 5 — REGIME STRESS: does the mandate survive spread compression?")
    print("-" * 100)
    print(f"  Historical delta-neutral funding capture median: 5-10% annualized per pair")
    print(f"  Current universe implied s̄: {current_s_avg_top_46*100:.2f}% (HIGH regime)")
    print()
    print(f"  {'config':<22}  {'current':>10}  {'moderate 10%':>14}  "
          f"{'normal 7%':>12}  {'conservative 5%':>17}")
    for L in leverages:
        for h in holds:
            label = f"L={L}, hold={hold_labels[h]}"
            row = [label]
            for s_regime in [current_s_avg_top_46, 0.10, 0.07, 0.05]:
                res = lifecycle_annualized_return(
                    per_pair_spread_apy=s_regime,
                    commitment_hold_h=h,
                    c_round_trip=c,
                    leverage=L,
                    alpha=alpha,
                    r_idle=r_idle,
                )
                cust = res["customer"]
                flag = "OK" if res["mandate_customer_ok"] else "FAIL"
                row.append(f"{cust*100:.2f}%[{flag}]")
            print(f"  {row[0]:<22}  {row[1]:>10}  {row[2]:>14}  {row[3]:>12}  {row[4]:>17}")

    # ---- Step 6: verdict ----
    print()
    print("=" * 100)
    print("VERDICT")
    print("=" * 100)

    # Find configurations that survive the "conservative 5%" stress
    survivors_5 = []
    survivors_7 = []
    survivors_10 = []
    for L in leverages:
        for h in holds:
            for stress, bucket in [(0.05, survivors_5), (0.07, survivors_7), (0.10, survivors_10)]:
                res = lifecycle_annualized_return(
                    per_pair_spread_apy=stress,
                    commitment_hold_h=h,
                    c_round_trip=c,
                    leverage=L,
                    alpha=alpha,
                    r_idle=r_idle,
                )
                if res["mandate_customer_ok"]:
                    bucket.append((L, hold_labels[h]))

    print(f"  Configurations that meet customer mandate @ s̄ = 5% (conservative): "
          f"{len(survivors_5)} / {len(leverages)*len(holds)}")
    for L, hl in survivors_5:
        print(f"    • L={L}, {hl}")
    print(f"  Configurations that meet customer mandate @ s̄ = 7% (normal): "
          f"{len(survivors_7)} / {len(leverages)*len(holds)}")
    for L, hl in survivors_7:
        print(f"    • L={L}, {hl}")
    print(f"  Configurations that meet customer mandate @ s̄ = 10% (moderate): "
          f"{len(survivors_10)} / {len(leverages)*len(holds)}")
    for L, hl in survivors_10:
        print(f"    • L={L}, {hl}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
