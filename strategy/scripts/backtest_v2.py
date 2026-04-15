"""
Empirical pair-trade backtest using ~42 days of Pacifica funding history.
Validates v2 strategy assumptions with REAL numbers.
"""
import json, os, statistics
from collections import defaultdict

HIST = os.path.expanduser('strategy/data/history')
syms = sorted([f[:-5] for f in os.listdir(HIST) if f.endswith('.json')])

def load(s):
    with open(f'{HIST}/{s}.json') as f:
        rows = json.load(f)
    rows.sort(key=lambda r: r['created_at'])
    return rows

data = {s: load(s) for s in syms}
common_ts = None
for s in syms:
    ts = {r['created_at'] for r in data[s]}
    common_ts = ts if common_ts is None else common_ts & ts
common_ts = sorted(common_ts)

def series(s, field='funding_rate'):
    m = {r['created_at']: float(r[field]) for r in data[s]}
    return [m[t] for t in common_ts]

# Hours covered
HOURS = len(common_ts)
YEARS = HOURS / (24*365)
print(f'Common hours: {HOURS}  ({HOURS/24:.1f} days, {YEARS:.4f} years)')
print()

FEE_MAKER = 0.00015  # per leg per side
FEE_TAKER = 0.00040

def backtest_pair(longA, shortB, threshold_in=0.10, threshold_out=0.03, fee_per_leg=FEE_MAKER):
    """
    Simulates a pair trade with hysteresis.
    threshold_in: enter trade when |spread_apy| > threshold_in (e.g. 0.10 = 10%)
    threshold_out: exit trade when |spread_apy| < threshold_out
    Returns: (gross_apy, net_apy, fee_drag_apy, n_cycles, time_in_trade_pct)
    """
    fA = series(longA)
    fB = series(shortB)
    spread_h = [(fB[i] - fA[i]) for i in range(HOURS)]  # positive = profitable to long A short B
    spread_apy = [s * 24 * 365 for s in spread_h]
    
    in_trade = False
    direction = 0  # +1 = long A short B, -1 = long B short A
    cycles = 0
    pnl_funding = 0.0  # in fraction of notional, per leg
    hours_in_trade = 0
    
    for i in range(HOURS):
        s = spread_apy[i]
        if not in_trade:
            if s > threshold_in:
                in_trade = True; direction = +1; cycles += 1
            elif s < -threshold_in:
                in_trade = True; direction = -1; cycles += 1
        else:
            # earn funding this hour
            sign = direction
            pnl_funding += sign * spread_h[i]
            hours_in_trade += 1
            if abs(s) < threshold_out or (direction == +1 and s < 0) or (direction == -1 and s > 0):
                in_trade = False; direction = 0
    
    # Fees: each cycle is 4 legs (open A, open B, close A, close B), each leg pays fee_per_leg on its notional
    fee_drag = cycles * 4 * fee_per_leg  # fraction of single-leg notional per year (well, per period)
    # Convert to APY: pnl_funding is over HOURS, scale to 1 year
    gross_apy = pnl_funding * (24*365 / HOURS)
    fee_apy = fee_drag / YEARS
    net_apy = gross_apy - fee_apy
    occ = hours_in_trade / HOURS
    return gross_apy, net_apy, fee_apy, cycles, occ

print('PAIR BACKTESTS (maker fee 0.015% per leg, hysteresis: enter >10% APY, exit <3% APY)')
print('=' * 110)
print(f"{'pair':<18}{'gross_apy':>12}{'net_apy':>12}{'fee_apy':>12}{'cycles':>10}{'occupancy':>12}{'fee_drag_pct':>16}")
candidates = [
    ('PAXG','XAU'),('BTC','ETH'),('BTC','SOL'),('BTC','BNB'),('SOL','ETH'),
    ('XAU','XAG'),('XAU','PAXG'),('PAXG','XAG'),
    ('EURUSD','USDJPY'),('USDJPY','EURUSD'),
    ('CL','NATGAS'),('COPPER','XAG'),
    ('SP500','NVDA'),('SP500','TSLA'),('NVDA','TSLA'),
    ('BTC','HYPE'),('HYPE','BTC'),
]
results = []
for a, b in candidates:
    if a not in data or b not in data: continue
    g, n, f, c, o = backtest_pair(a, b)
    drag_pct = (f / g * 100) if g != 0 else 0
    pname = f'{a}/{b}'
    results.append((pname, g, n, f, c, o))
    print(f'{pname:<18}{g*100:>11.2f}%{n*100:>11.2f}%{f*100:>11.2f}%{c:>10}{o*100:>11.1f}%{drag_pct:>15.1f}%')

print()
print('SAME PAIRS WITH TAKER FEES (0.040% per leg)')
print('=' * 110)
print(f"{'pair':<18}{'gross_apy':>12}{'net_apy':>12}{'fee_apy':>12}{'cycles':>10}{'occupancy':>12}")
for a, b in candidates:
    if a not in data or b not in data: continue
    g, n, f, c, o = backtest_pair(a, b, fee_per_leg=FEE_TAKER)
    pname = f'{a}/{b}'
    print(f'{pname:<18}{g*100:>11.2f}%{n*100:>11.2f}%{f*100:>11.2f}%{c:>10}{o*100:>11.1f}%')

print()
print('SENSITIVITY: looser threshold (enter >5%, exit <2%) maker fees')
print('=' * 110)
print(f"{'pair':<18}{'gross_apy':>12}{'net_apy':>12}{'fee_apy':>12}{'cycles':>10}{'occupancy':>12}")
for a, b in candidates:
    if a not in data or b not in data: continue
    g, n, f, c, o = backtest_pair(a, b, threshold_in=0.05, threshold_out=0.02)
    pname = f'{a}/{b}'
    print(f'{pname:<18}{g*100:>11.2f}%{n*100:>11.2f}%{f*100:>11.2f}%{c:>10}{o*100:>11.1f}%')

print()
print('SENSITIVITY: tighter threshold (enter >20%, exit <5%) maker fees')
print('=' * 110)
print(f"{'pair':<18}{'gross_apy':>12}{'net_apy':>12}{'fee_apy':>12}{'cycles':>10}{'occupancy':>12}")
for a, b in candidates:
    if a not in data or b not in data: continue
    g, n, f, c, o = backtest_pair(a, b, threshold_in=0.20, threshold_out=0.05)
    pname = f'{a}/{b}'
    print(f'{pname:<18}{g*100:>11.2f}%{n*100:>11.2f}%{f*100:>11.2f}%{c:>10}{o*100:>11.1f}%')
