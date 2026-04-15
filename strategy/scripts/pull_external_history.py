"""
pull_external_history.py — fetch historical hourly funding rates from Hyperliquid
and Backpack for symbols that overlap with Pacifica's universe. Used by
analyze_persistence_historical.py to backtest v3 cross-venue arb without
waiting for the live poller to accumulate days of data.

Outputs:
  data/history_hyperliquid/{SYMBOL}.json
  data/history_backpack/{SYMBOL}.json

Each file is a list of {time_ms, funding_rate} sorted ascending.
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timezone, timedelta

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HL_DIR = os.path.join(ROOT, "data", "history_hyperliquid")
BP_DIR = os.path.join(ROOT, "data", "history_backpack")

# Pacifica → Hyperliquid: bare ticker
# Pacifica → Backpack: BASE_USDC_PERP
SYMBOLS = [
    "BTC", "ETH", "SOL", "BNB", "HYPE", "PAXG", "STRK", "2Z",
    "TAO", "WIF", "WLD", "LDO", "JUP", "ADA", "DOGE", "TRUMP",
    "ICP", "NEAR", "ZRO", "ENA", "ZEC", "ARB", "AVAX", "LINK",
    "XRP", "SUI", "AAVE", "kBONK", "kPEPE", "WLFI", "FARTCOIN",
    "PENGU", "PUMP", "VIRTUAL", "MON", "PIPPIN",
]

# Aim for ~60 days of history
TARGET_HOURS = 24 * 60
NOW_MS = int(time.time() * 1000)
START_MS = NOW_MS - TARGET_HOURS * 3600 * 1000

USER_AGENT = "Mozilla/5.0 (DolStrategy/1.0)"
TIMEOUT = 30


def http_get_json(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return json.load(r)


def http_post_json(url: str, body: dict):
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        url, data=data,
        headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return json.load(r)


def pull_hyperliquid(coin: str) -> list:
    """Paginate Hyperliquid fundingHistory forward in time. Returns list of
    {time_ms, funding_rate}."""
    all_rows = []
    cursor = START_MS
    safety = 0
    while cursor < NOW_MS and safety < 20:
        try:
            batch = http_post_json(
                "https://api.hyperliquid.xyz/info",
                {"type": "fundingHistory", "coin": coin, "startTime": cursor},
            )
        except urllib.error.HTTPError as e:
            print(f"  HL {coin} HTTP {e.code}", file=sys.stderr)
            return all_rows
        except Exception as e:
            print(f"  HL {coin} error: {e}", file=sys.stderr)
            return all_rows
        if not isinstance(batch, list) or not batch:
            break
        new_rows = [{"time_ms": r["time"], "funding_rate": float(r["fundingRate"])} for r in batch]
        # de-dup against existing
        seen_ts = {r["time_ms"] for r in all_rows}
        added = [r for r in new_rows if r["time_ms"] not in seen_ts]
        all_rows.extend(added)
        last_ts = batch[-1]["time"]
        if last_ts <= cursor:
            break
        cursor = last_ts + 1
        safety += 1
        time.sleep(0.15)
    all_rows.sort(key=lambda r: r["time_ms"])
    return all_rows


def pull_backpack(symbol_perp: str) -> list:
    """Backpack supports limit=10000 in one request. Returns rows sorted ascending."""
    try:
        data = http_get_json(
            f"https://api.backpack.exchange/api/v1/fundingRates?symbol={symbol_perp}&limit=10000"
        )
    except urllib.error.HTTPError as e:
        print(f"  BP {symbol_perp} HTTP {e.code}", file=sys.stderr)
        return []
    except Exception as e:
        print(f"  BP {symbol_perp} error: {e}", file=sys.stderr)
        return []
    if not isinstance(data, list):
        return []
    rows = []
    for r in data:
        try:
            ts = int(datetime.fromisoformat(r["intervalEndTimestamp"].replace("Z", "+00:00"))
                     .replace(tzinfo=timezone.utc).timestamp() * 1000)
        except Exception:
            try:
                ts = int(datetime.strptime(r["intervalEndTimestamp"], "%Y-%m-%dT%H:%M:%S")
                         .replace(tzinfo=timezone.utc).timestamp() * 1000)
            except Exception:
                continue
        rows.append({"time_ms": ts, "funding_rate": float(r["fundingRate"])})
    rows.sort(key=lambda r: r["time_ms"])
    # filter to target window
    rows = [r for r in rows if r["time_ms"] >= START_MS]
    return rows


def main():
    os.makedirs(HL_DIR, exist_ok=True)
    os.makedirs(BP_DIR, exist_ok=True)

    print(f"Target window: last {TARGET_HOURS} hours ({TARGET_HOURS/24:.0f} days)")
    print(f"  start: {datetime.fromtimestamp(START_MS/1000, tz=timezone.utc)}")
    print(f"  end:   {datetime.fromtimestamp(NOW_MS/1000, tz=timezone.utc)}")
    print()

    print(f"Pulling {len(SYMBOLS)} symbols from Hyperliquid + Backpack...")
    print()

    summary = []
    for sym in SYMBOLS:
        # Hyperliquid uses bare ticker. Skip "k"-prefixed (kBONK / kPEPE) since HL syntax differs.
        hl_coin = sym
        hl_rows = []
        if not sym.startswith("k"):
            try:
                hl_rows = pull_hyperliquid(hl_coin)
            except Exception as e:
                print(f"  HL {sym} fatal: {e}", file=sys.stderr)
        if hl_rows:
            with open(os.path.join(HL_DIR, f"{sym}.json"), "w") as f:
                json.dump(hl_rows, f)

        bp_sym = f"{sym}_USDC_PERP"
        bp_rows = pull_backpack(bp_sym)
        if bp_rows:
            with open(os.path.join(BP_DIR, f"{sym}.json"), "w") as f:
                json.dump(bp_rows, f)

        line = f"{sym:<12} HL={len(hl_rows):>5} BP={len(bp_rows):>5}"
        if hl_rows:
            line += f"  HL_first={datetime.fromtimestamp(hl_rows[0]['time_ms']/1000, tz=timezone.utc).strftime('%m-%d %H:%M')}"
        if bp_rows:
            line += f"  BP_first={datetime.fromtimestamp(bp_rows[0]['time_ms']/1000, tz=timezone.utc).strftime('%m-%d %H:%M')}"
        print(line)
        summary.append((sym, len(hl_rows), len(bp_rows)))

    print()
    print(f"Total: HL coverage on {sum(1 for s in summary if s[1]>0)}/{len(SYMBOLS)} symbols")
    print(f"       BP coverage on {sum(1 for s in summary if s[2]>0)}/{len(SYMBOLS)} symbols")
    return 0


if __name__ == "__main__":
    sys.exit(main())
