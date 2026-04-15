# scripts/ — Cross-venue funding data collection (v3 Phase 1)

## What's here

- **`poll_aggregated.py`** — long-running daemon. Polls Pacifica's `/api/v1/funding_rate/aggregated` every 5 minutes and writes funding rates from 6 supported venues (pacifica, backpack, binance, bybit, hyperliquid, lighter) to `data/cross_venue_funding.sqlite`. Other venues returned by the API (paradex, aster, okx, bitget, coinbase) are filtered out per design decision because the Pacifica funding page UI does not display them.
- **`analyze_persistence.py`** — interim and final analysis. Runs anytime over whatever data has been collected so far. Use `--dex-only` to restrict counter venues to KYC-free DEXes (Backpack/Hyperliquid/Lighter), which is the actionable universe for the Dol vault.
- **`backtest_v2.py`** — earlier pair-trade backtest (Pacifica-only, β-hedged). Kept for audit trail; v3 strategy uses cross-venue same-asset arb instead.

## Daemon — start, status, stop

### Start (background, persistent)
```bash
cd strategy
mkdir -p logs
PYTHONIOENCODING=utf-8 nohup .venv/Scripts/python.exe scripts/poll_aggregated.py \
    > logs/poll_aggregated.log 2>&1 &
```

The daemon writes to `data/cross_venue_funding.sqlite` (WAL mode, safe for concurrent reads while writing).

### Check status
```bash
tasklist | grep -i python                                # is python running?
tail -10 strategy/logs/poll_aggregated.log  # recent poll output
strategy/.venv/Scripts/python.exe -c "
import sqlite3
c = sqlite3.connect('strategy/data/cross_venue_funding.sqlite')
print('polls:', c.execute('SELECT COUNT(*) FROM poll_runs').fetchone()[0])
print('rows:',  c.execute('SELECT COUNT(*) FROM funding_aggregated').fetchone()[0])
"
```

### Stop
Find the python PID via `tasklist | grep python.exe` and `taskkill /PID <pid> /F`, or close the parent shell that launched it.

### Restart after reboot / session end
The daemon does NOT auto-restart. After any system or shell restart, re-run the start command above. The sqlite DB persists — restarted daemon will append new rows alongside existing data.

## Analyze (interim or final)

Run anytime — the script handles partial windows gracefully.

```bash
# All 6 venues, including CEXes (informational)
strategy/.venv/Scripts/python.exe \
    strategy/scripts/analyze_persistence.py

# DEX-only counter venues (actionable universe for Dol vault)
strategy/.venv/Scripts/python.exe \
    strategy/scripts/analyze_persistence.py --dex-only

# Single symbol deep-dive
strategy/.venv/Scripts/python.exe \
    strategy/scripts/analyze_persistence.py --symbol HYPE

# Custom occupancy thresholds
strategy/.venv/Scripts/python.exe \
    strategy/scripts/analyze_persistence.py --thresholds 5,15,40,80
```

Output sections:
1. **Per-venue reliability** — % of polls each venue returned data
2. **Per-symbol occupancy** — % of polls each symbol's spread exceeded each threshold (5, 10, 30, 50, 100% APY)
3. **Spread compression** — early-half vs late-half mean spread per symbol (detects shrinking opportunity)
4. **Counter-venue distribution** — which venue is the best counter for each top symbol
5. **Portfolio summary** — average # symbols above each threshold per poll (bottom-line capacity signal)

## Phase 1 deliverable target (per v3 design §8)

After ~7 days of continuous polling (~2,000 polls), `analyze_persistence.py --dex-only` should show:
- **Per-venue reliability** ≥ 98% for all 4 DEX venues (pacifica + 3 counters)
- **At least 8 symbols with >30% APY occupancy ≥ 50%** (i.e., the symbol is in arb-tradable territory more than half the time)
- **Spread compression delta** in single digits (<10% absolute drift in mean spread between early and late halves), confirming spreads are not just transient
- **Average # symbols above 30% APY per poll ≥ 8** (enough breadth for an 8-pair portfolio)

If these gates are met, v3 strategy is validated and Phase 2 (counter-venue API integration scoping for Backpack + Hyperliquid + Lighter) can begin.
