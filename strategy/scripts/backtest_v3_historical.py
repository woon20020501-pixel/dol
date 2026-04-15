"""
backtest_v3_historical.py — proper v3 cross-venue same-asset arb backtest using
60 days of merged Pacifica + Hyperliquid + Backpack hourly funding history.

Strategy logic per pair:
  - Each hour compute spread = best_counter_venue_funding - pacifica_funding (per hour)
  - Smoothed signal = rolling mean over SMOOTH_HOURS
  - Enter long Pacifica + short counter (or vice versa) when |smoothed_apy| > ENTER_APY
  - Hold at least MIN_HOLD_HOURS, then exit when |smoothed_apy| < EXIT_APY
  - Each cycle pays 4 leg fees: 2 on Pacifica (open + close) + 2 on counter (open + close)

Reports per-pair and equal-weighted portfolio NET APY for top symbols.

Fees:
  - pacifica   maker 0.015%, taker 0.040%
  - hyperliquid maker 0.025%, taker 0.050%  (approx — confirm via HL docs)
  - backpack   maker 0.020%, taker 0.050%  (approx)
  - assume MAKER ONLY for the strategy (limit orders), entries fall through if missed
"""
import argparse
import os
import sqlite3
import statistics
import sys
from collections import defaultdict
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_DB = os.path.join(ROOT, "data", "historical_cross_venue.sqlite")

# Maker fees per leg, conservative
FEE_PER_LEG = {
    "pacifica":    0.00015,
    "hyperliquid": 0.00025,
    "backpack":    0.00020,
    "lighter":     0.00020,
}
DEX_VENUES = {"pacifica", "backpack", "hyperliquid", "lighter"}


def smooth(x, window):
    out = []
    for i in range(len(x)):
        lo = max(0, i - window + 1)
        out.append(sum(x[lo:i+1]) / (i - lo + 1))
    return out


def load_history(db_path: str):
    """Returns:
       hours: sorted list of hour_ms
       data: {symbol: {hour_ms: {venue: rate}}}
    """
    conn = sqlite3.connect(db_path)
    rows = conn.execute(
        "SELECT timestamp_ms, symbol, venue, funding_rate FROM funding_aggregated "
        "WHERE venue IN ('pacifica','backpack','hyperliquid','lighter') "
        "ORDER BY timestamp_ms"
    ).fetchall()
    data = defaultdict(lambda: defaultdict(dict))
    hours = set()
    for ts, sym, venue, rate in rows:
        data[sym][ts][venue] = rate
        hours.add(ts)
    return sorted(hours), data


def backtest_pair(symbol_data, hours, enter_apy, exit_apy, smooth_h, min_hold_h):
    """For one symbol, simulate the cross-venue arb against the BEST available
    DEX counter venue at each hour. Returns:
      funding_apy, fee_apy, net_apy, cycles, occupancy, n_obs
    """
    # Per hour, compute (pacifica_rate, best_counter_venue, best_counter_rate)
    series = []
    for h in hours:
        if h not in symbol_data:
            series.append(None)
            continue
        venues_at_h = symbol_data[h]
        if "pacifica" not in venues_at_h:
            series.append(None)
            continue
        pac = venues_at_h["pacifica"]
        best = None
        for v, r in venues_at_h.items():
            if v == "pacifica" or v not in DEX_VENUES:
                continue
            diff = r - pac
            if best is None or abs(diff) > abs(best[1]):
                best = (v, diff, r)
        if best is None:
            series.append(None)
        else:
            series.append((pac, best[0], best[1], best[2]))

    # Smoothed spread signal in APY %
    raw_spread_h = [s[2] if s else 0.0 for s in series]  # spread = counter - pacifica
    smoothed = smooth(raw_spread_h, smooth_h)

    in_trade = False
    direction = 0       # +1 = long pacifica + short counter, -1 = short pacifica + long counter
    entry_hour_idx = -1
    pinned_counter = None
    cycles = 0
    funding_pnl = 0.0   # in fraction of single-leg notional
    hours_in = 0
    pac_fee = FEE_PER_LEG["pacifica"]

    valid_hours = sum(1 for s in series if s is not None)

    for i, s in enumerate(series):
        if s is None:
            # missing data: hold position as-is, don't accrue, don't exit
            continue
        pac_rate, counter_venue, spread_h_val, counter_rate = s
        smoothed_apy = smoothed[i] * 24 * 365

        # Accrue funding if in trade
        if in_trade and pinned_counter == counter_venue:
            # direction +1: long pacifica receives -pac_rate, short counter receives +counter_rate
            # net per hour on $1 long: -pac_rate + counter_rate = spread_h_val (sign-aware)
            funding_pnl += direction * spread_h_val
            hours_in += 1
        elif in_trade and pinned_counter != counter_venue:
            # counter venue switched — close current and reopen on new counter
            # close fees
            close_fee = pac_fee + FEE_PER_LEG.get(pinned_counter, 0.00025)
            cycles += 0  # we count this as a single cycle continuation, fees added below
            # Re-open on new counter
            pinned_counter = counter_venue
            # add open fees
            open_fee = pac_fee + FEE_PER_LEG.get(counter_venue, 0.00025)
            # both close and open are extra fees
            funding_pnl -= (close_fee + open_fee)
            cycles += 1  # counter switch counts as a partial cycle
            funding_pnl += direction * spread_h_val
            hours_in += 1

        held = i - entry_hour_idx if in_trade else 0
        if not in_trade:
            if smoothed_apy > enter_apy:
                in_trade = True; direction = +1
                entry_hour_idx = i; pinned_counter = counter_venue
                cycles += 1
            elif smoothed_apy < -enter_apy:
                in_trade = True; direction = -1
                entry_hour_idx = i; pinned_counter = counter_venue
                cycles += 1
        else:
            if held >= min_hold_h:
                exit_signal = (
                    (direction == +1 and smoothed_apy < exit_apy) or
                    (direction == -1 and smoothed_apy > -exit_apy)
                )
                if exit_signal:
                    in_trade = False; direction = 0; entry_hour_idx = -1; pinned_counter = None

    years = valid_hours / (24*365) if valid_hours else 1.0
    fee_drag = cycles * 2 * (pac_fee + 0.00025)  # average counter fee 0.025%, 2 sides per cycle
    funding_apy = funding_pnl / years
    fee_apy = fee_drag / years
    net_apy = funding_apy - fee_apy
    occ = hours_in / valid_hours if valid_hours else 0
    return funding_apy, fee_apy, net_apy, cycles, occ, valid_hours


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--enter", type=float, default=20.0, help="enter threshold APY %%")
    ap.add_argument("--exit", type=float, default=5.0, help="exit threshold APY %%")
    ap.add_argument("--smooth", type=int, default=24, help="smoothing window hours")
    ap.add_argument("--hold", type=int, default=24, help="min hold hours")
    args = ap.parse_args()

    enter_apy = args.enter / 100.0
    exit_apy = args.exit / 100.0

    print(f"DB: {args.db}")
    print(f"Strategy: enter>{args.enter}%, exit<{args.exit}%, smooth={args.smooth}h, hold>={args.hold}h")
    print()

    hours, data = load_history(args.db)
    span_hours = (hours[-1] - hours[0]) / 3600000 if len(hours) > 1 else 0
    print(f"Window: {datetime.fromtimestamp(hours[0]/1000, tz=timezone.utc):%Y-%m-%d} -> "
          f"{datetime.fromtimestamp(hours[-1]/1000, tz=timezone.utc):%Y-%m-%d} "
          f"({span_hours:.0f}h, {span_hours/24:.1f}d)")
    print(f"Total symbols with data: {len(data)}")
    print()

    print("PER-SYMBOL BACKTEST  cross-venue Pacifica-anchored arb, DEX counters only, MAKER fees")
    print("=" * 110)
    print(f"{'symbol':<12}{'n_obs':>8}{'fund_apy':>12}{'fee_apy':>11}{'NET_apy':>11}{'cycles':>9}{'occupancy':>11}")

    results = []
    for sym in sorted(data.keys()):
        f, fee, n, c, occ, n_obs = backtest_pair(
            data[sym], hours, enter_apy, exit_apy, args.smooth, args.hold
        )
        if n_obs < 100:
            continue
        results.append((n, sym, f, fee, c, occ, n_obs))

    results.sort(reverse=True)
    for n, sym, f, fee, c, occ, n_obs in results:
        print(f"{sym:<12}{n_obs:>8}{f*100:>11.2f}%{fee*100:>10.2f}%{n*100:>10.2f}%{c:>9}{occ*100:>10.1f}%")

    print()
    profitable = [r for r in results if r[0] > 0]
    print(f"PROFITABLE PAIRS: {len(profitable)}/{len(results)}")
    if profitable:
        top8 = profitable[:8]
        print()
        print("TOP 8 BY NET APY (equal-weighted portfolio simulation):")
        print("-" * 70)
        net_sum = 0
        for n, sym, f, fee, c, occ, n_obs in top8:
            print(f"  {sym:<12} fund={f*100:>7.2f}% fee={fee*100:>5.2f}% NET={n*100:>7.2f}% occ={occ*100:>5.1f}% cycles={c}")
            net_sum += n
        print()
        print(f"  Sum of NET APYs over top 8 pairs: {net_sum*100:.2f}%")
        print(f"  Equal-weighted portfolio APY (per dollar of TOTAL deployed across 8 pairs): {net_sum*100/8:.2f}%")
        print(f"  Per-pair average NET: {net_sum*100/8:.2f}%")
        print()
        print(f"  Top 12 portfolio sum: {sum(r[0] for r in profitable[:12])*100:.2f}%")
        print(f"  Top 12 equal-weighted: {sum(r[0] for r in profitable[:12])*100/12:.2f}%")
        print()
        print(f"  Top 16 portfolio sum: {sum(r[0] for r in profitable[:16])*100:.2f}%")
        print(f"  Top 16 equal-weighted: {sum(r[0] for r in profitable[:16])*100/16:.2f}%")


if __name__ == "__main__":
    sys.exit(main() or 0)
