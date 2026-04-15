"""
analyze_persistence.py — interim and final analysis of cross_venue_funding.sqlite.

Computes the four signals design goal was after collecting 7 days of cross-venue
funding data (or runs interim on partial data):

  1. Spread variance per symbol over time (hourly stddev of best Pacifica-anchored arb spread)
  2. Average occupancy per symbol (% of polls where best spread > THRESHOLDS)
  3. Per-venue reliability (response presence rate per poll, per venue)
  4. Spread compression trend (rolling mean of best spread over time, slope)

Run:
  python scripts/analyze_persistence.py
  # filter to specific symbol:
  python scripts/analyze_persistence.py --symbol CL
  # adjust threshold for occupancy bucket:
  python scripts/analyze_persistence.py --thresholds 10,20,30,50,100
"""
import argparse
import os
import sqlite3
import statistics
import sys
from collections import defaultdict
from datetime import datetime, timezone

DEFAULT_DB = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "data",
    "cross_venue_funding.sqlite",
)
SUPPORTED_VENUES = ["pacifica", "backpack", "binance", "bybit", "hyperliquid", "lighter"]
DEX_VENUES = {"pacifica", "backpack", "hyperliquid", "lighter"}  # KYC-free, usable by Dol vault


def iso(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")


def fetch_all(conn: sqlite3.Connection):
    rows = conn.execute(
        "SELECT timestamp_ms, symbol, venue, funding_rate FROM funding_aggregated ORDER BY timestamp_ms"
    ).fetchall()
    poll_runs = conn.execute(
        "SELECT timestamp_ms, success, n_symbols, n_rows, venues_seen FROM poll_runs ORDER BY timestamp_ms"
    ).fetchall()
    return rows, poll_runs


def index_by_poll(rows):
    """Returns {ts_ms: {symbol: {venue: rate}}}."""
    out = defaultdict(lambda: defaultdict(dict))
    for ts, sym, venue, rate in rows:
        out[ts][sym][venue] = rate
    return out


def best_pacifica_spread(symbol_rates: dict, dex_only: bool = False) -> tuple:
    """Returns (spread_h, counter_venue, side) for the largest |spread| with Pacifica
    as one leg. If dex_only=True, restrict counter to KYC-free DEX venues only."""
    pac = symbol_rates.get("pacifica")
    if pac is None:
        return None
    best = None
    for venue, rate in symbol_rates.items():
        if venue == "pacifica":
            continue
        if dex_only and venue not in DEX_VENUES:
            continue
        diff = rate - pac
        if best is None or abs(diff) > abs(best[0]):
            best = (diff, venue, "short_other_long_pacifica" if diff > 0 else "short_pacifica_long_other")
    return best


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--symbol", help="filter to single symbol")
    ap.add_argument("--thresholds", default="10,20,30,50,100",
                    help="comma list of APY%% thresholds for occupancy")
    ap.add_argument("--dex-only", action="store_true",
                    help="restrict counter venues to KYC-free DEX (backpack, hyperliquid, lighter); "
                         "this is the actionable universe for the Dol vault")
    ap.add_argument("--db", default=DEFAULT_DB,
                    help="path to sqlite db (default: data/cross_venue_funding.sqlite)")
    args = ap.parse_args()
    DB_PATH = args.db
    thresholds = [float(t) for t in args.thresholds.split(",")]

    if not os.path.exists(DB_PATH):
        print(f"ERROR: db not found at {DB_PATH}")
        return 1

    conn = sqlite3.connect(DB_PATH)
    rows, poll_runs = fetch_all(conn)
    if not rows:
        print("no rows yet")
        return 0
    if args.symbol:
        rows = [r for r in rows if r[1] == args.symbol]

    by_poll = index_by_poll(rows)
    polls_sorted = sorted(by_poll.keys())
    n_polls = len(polls_sorted)
    span_h = (polls_sorted[-1] - polls_sorted[0]) / 1000 / 3600
    print("=" * 100)
    print(f"DATA WINDOW: {iso(polls_sorted[0])} -> {iso(polls_sorted[-1])}")
    print(f"polls: {n_polls}, span: {span_h:.2f} hours, total rows: {len(rows)}")
    print(f"counter universe: {'DEX-only (Backpack/Hyperliquid/Lighter)' if args.dex_only else 'ALL 6 venues (incl. CEXes Binance/Bybit)'}")
    print("=" * 100)
    print()

    # -------- 1. per-venue reliability --------
    print("(1) PER-VENUE RELIABILITY  presence rate per poll")
    print("-" * 60)
    venue_present_count = defaultdict(int)
    for ts in polls_sorted:
        venues_in_poll = set()
        for sym_rates in by_poll[ts].values():
            venues_in_poll.update(sym_rates.keys())
        for v in venues_in_poll:
            venue_present_count[v] += 1
    print(f"{'venue':<14}{'present_polls':>16}{'rate':>10}")
    for v in SUPPORTED_VENUES:
        cnt = venue_present_count.get(v, 0)
        rate = cnt / n_polls * 100 if n_polls else 0
        print(f"{v:<14}{cnt:>16}{rate:>9.1f}%")
    print()

    # -------- 2. per-symbol Pacifica-anchored best spread time series --------
    symbol_spread_series = defaultdict(list)  # {sym: [(ts, spread_apy, counter_venue)]}
    for ts in polls_sorted:
        for sym, sym_rates in by_poll[ts].items():
            best = best_pacifica_spread(sym_rates, dex_only=args.dex_only)
            if best is None:
                continue
            spread_apy = best[0] * 24 * 365 * 100
            symbol_spread_series[sym].append((ts, spread_apy, best[1]))

    # -------- 3. occupancy (% polls with |spread| > threshold) --------
    print("(2) OCCUPANCY  % of polls with |best Pacifica-anchored spread| > threshold (per symbol, top 25 by max spread)")
    print("-" * 100)
    header = f"{'symbol':<10}{'n_obs':>8}{'mean_apy':>11}{'max_apy':>11}{'std_apy':>11}"
    for t in thresholds:
        header += f"{'>'+str(int(t))+'%':>10}"
    print(header)
    rankings = []
    for sym, series in symbol_spread_series.items():
        if len(series) < 1:
            continue
        spreads = [abs(s[1]) for s in series]
        mean_s = statistics.mean(spreads)
        max_s = max(spreads)
        std_s = statistics.stdev(spreads) if len(spreads) > 1 else 0.0
        occ = {t: sum(1 for s in spreads if s > t) / len(spreads) * 100 for t in thresholds}
        rankings.append((max_s, sym, len(series), mean_s, max_s, std_s, occ))
    rankings.sort(reverse=True)
    for _, sym, n, mean_s, max_s, std_s, occ in rankings[:25]:
        line = f"{sym:<10}{n:>8}{mean_s:>10.1f}%{max_s:>10.1f}%{std_s:>10.1f}%"
        for t in thresholds:
            line += f"{occ[t]:>9.1f}%"
        print(line)
    print()

    # -------- 4. spread compression / mean trend --------
    print("(3) SPREAD COMPRESSION  early-half vs late-half mean (top 15 by sample count)")
    print("-" * 100)
    print(f"{'symbol':<10}{'n_obs':>8}{'early_mean':>13}{'late_mean':>13}{'delta':>11}{'trend':>10}")
    compress = []
    for sym, series in symbol_spread_series.items():
        if len(series) < 4:
            continue
        spreads = [abs(s[1]) for s in series]
        mid = len(spreads) // 2
        early = statistics.mean(spreads[:mid]) if mid else 0
        late = statistics.mean(spreads[mid:]) if mid else 0
        delta = late - early
        compress.append((len(series), sym, early, late, delta))
    compress.sort(reverse=True)
    for n, sym, e, l, d in compress[:15]:
        trend = "↓compress" if d < -2 else ("↑expand" if d > 2 else "stable")
        print(f"{sym:<10}{n:>8}{e:>12.1f}%{l:>12.1f}%{d:>10.1f}%  {trend}")
    print()

    # -------- 5. counter-venue distribution per top symbol --------
    print("(4) COUNTER-VENUE DISTRIBUTION  which venue is the best counter per symbol (top 15)")
    print("-" * 100)
    print(f"{'symbol':<10}{'n_obs':>8}  counter_venue counts (% of polls)")
    for _, sym, _, _, _, _, _ in rankings[:15]:
        series = symbol_spread_series[sym]
        counts = defaultdict(int)
        for _, _, cv in series:
            counts[cv] += 1
        total = sum(counts.values())
        bits = sorted(counts.items(), key=lambda kv: -kv[1])
        bit_str = "  ".join(f"{cv}:{c/total*100:.0f}%" for cv, c in bits)
        print(f"{sym:<10}{total:>8}  {bit_str}")
    print()

    # -------- 6. portfolio-level summary --------
    print("(5) PORTFOLIO-LEVEL SUMMARY")
    print("-" * 60)
    pacifica_spreads_per_poll = []
    for ts in polls_sorted:
        spreads = []
        for sym, sym_rates in by_poll[ts].items():
            best = best_pacifica_spread(sym_rates, dex_only=args.dex_only)
            if best is None:
                continue
            spreads.append(abs(best[0] * 24 * 365 * 100))
        pacifica_spreads_per_poll.append(spreads)

    above10 = [sum(1 for s in sp if s > 10) for sp in pacifica_spreads_per_poll]
    above30 = [sum(1 for s in sp if s > 30) for sp in pacifica_spreads_per_poll]
    above50 = [sum(1 for s in sp if s > 50) for sp in pacifica_spreads_per_poll]
    print(f"avg # symbols with spread > 10% APY per poll: {statistics.mean(above10):.1f}")
    print(f"avg # symbols with spread > 30% APY per poll: {statistics.mean(above30):.1f}")
    print(f"avg # symbols with spread > 50% APY per poll: {statistics.mean(above50):.1f}")
    if pacifica_spreads_per_poll[-1]:
        latest = pacifica_spreads_per_poll[-1]
        print(f"latest poll: {len(latest)} symbols, mean {statistics.mean(latest):.1f}% APY, max {max(latest):.1f}% APY")
    print()
    print("Strategy implication preview:")
    avg_above_30 = statistics.mean(above30)
    if avg_above_30 >= 8:
        print(f"  ✓ {avg_above_30:.0f} symbols typically above 30% APY — comfortable for 8-pair portfolio")
    elif avg_above_30 >= 5:
        print(f"  ⚠ {avg_above_30:.0f} symbols typically above 30% APY — workable but tight")
    else:
        print(f"  ✗ only {avg_above_30:.0f} symbols typically above 30% APY — need lower threshold or wider universe")


if __name__ == "__main__":
    sys.exit(main() or 0)
