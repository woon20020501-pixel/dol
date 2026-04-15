"""
poll_aggregated.py — Cross-venue funding rate poller for v3 strategy validation.

Polls Pacifica's /api/v1/funding_rate/aggregated every POLL_INTERVAL_SEC and
appends rows to data/cross_venue_funding.sqlite. Only retains the 6 venues that
the Pacifica funding page UI actually uses (pacifica, backpack, binance, bybit,
hyperliquid, lighter); other venues returned by the API are filtered out because
the Pacifica funding page UI does not display them.

Designed to run as a long-lived daemon for 7+ days to collect the baseline
needed by analyze_persistence.py.

Schema:
  CREATE TABLE funding_aggregated (
    timestamp_ms INTEGER NOT NULL,    -- when the poller wrote the row
    symbol       TEXT NOT NULL,
    venue        TEXT NOT NULL,
    funding_rate REAL NOT NULL,        -- per-hour rate, signed
    PRIMARY KEY (timestamp_ms, symbol, venue)
  )
  CREATE TABLE poll_runs (
    timestamp_ms INTEGER PRIMARY KEY,  -- when poll completed
    success      INTEGER NOT NULL,
    n_symbols    INTEGER NOT NULL,
    n_rows       INTEGER NOT NULL,
    venues_seen  TEXT NOT NULL,        -- comma-separated set of venues observed
    error        TEXT
  )

Run:
  python scripts/poll_aggregated.py
  # or with custom interval (seconds):
  POLL_INTERVAL_SEC=300 python scripts/poll_aggregated.py
"""
import json
import os
import sqlite3
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timezone

ENDPOINT = "https://api.pacifica.fi/api/v1/funding_rate/aggregated"
SUPPORTED_VENUES = {"pacifica", "backpack", "binance", "bybit", "hyperliquid", "lighter"}
POLL_INTERVAL_SEC = int(os.environ.get("POLL_INTERVAL_SEC", "300"))
DB_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "data",
    "cross_venue_funding.sqlite",
)
USER_AGENT = "Mozilla/5.0 (DolStrategy/1.0 +strategy)"
HTTP_TIMEOUT = 20
MAX_RETRIES_PER_POLL = 3
RETRY_BACKOFF_SEC = 5


def init_db(path: str) -> sqlite3.Connection:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    conn = sqlite3.connect(path, isolation_level=None)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS funding_aggregated (
            timestamp_ms INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            venue TEXT NOT NULL,
            funding_rate REAL NOT NULL,
            PRIMARY KEY (timestamp_ms, symbol, venue)
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS poll_runs (
            timestamp_ms INTEGER PRIMARY KEY,
            success INTEGER NOT NULL,
            n_symbols INTEGER NOT NULL,
            n_rows INTEGER NOT NULL,
            venues_seen TEXT NOT NULL,
            error TEXT
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_funding_symbol_ts ON funding_aggregated (symbol, timestamp_ms)"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_funding_venue_ts ON funding_aggregated (venue, timestamp_ms)"
    )
    return conn


def fetch_aggregated() -> list:
    last_err = None
    for attempt in range(MAX_RETRIES_PER_POLL):
        try:
            req = urllib.request.Request(ENDPOINT, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as r:
                payload = json.load(r)
            if not payload.get("success"):
                raise RuntimeError(f"API error: {payload.get('error')}")
            return payload["data"]
        except (urllib.error.URLError, json.JSONDecodeError, RuntimeError) as e:
            last_err = e
            if attempt < MAX_RETRIES_PER_POLL - 1:
                time.sleep(RETRY_BACKOFF_SEC * (attempt + 1))
    raise RuntimeError(f"fetch_aggregated failed after {MAX_RETRIES_PER_POLL} attempts: {last_err}")


def store_snapshot(conn: sqlite3.Connection, ts_ms: int, data: list) -> tuple:
    rows = []
    venues_seen = set()
    for entry in data:
        sym = entry.get("symbol")
        rates = entry.get("rates", {}) or {}
        for venue, rate in rates.items():
            if venue not in SUPPORTED_VENUES:
                continue
            try:
                rate_f = float(rate)
            except (TypeError, ValueError):
                continue
            rows.append((ts_ms, sym, venue, rate_f))
            venues_seen.add(venue)
    if rows:
        conn.executemany(
            "INSERT OR IGNORE INTO funding_aggregated VALUES (?,?,?,?)",
            rows,
        )
    return len(rows), len(data), venues_seen


def log_poll(conn: sqlite3.Connection, ts_ms: int, success: bool, n_symbols: int,
             n_rows: int, venues_seen: set, error: str = None) -> None:
    conn.execute(
        "INSERT OR REPLACE INTO poll_runs VALUES (?,?,?,?,?,?)",
        (ts_ms, 1 if success else 0, n_symbols, n_rows,
         ",".join(sorted(venues_seen)), error),
    )


def now_ms() -> int:
    return int(time.time() * 1000)


def iso_utc(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")


def main() -> int:
    print(f"[poll_aggregated] starting; interval={POLL_INTERVAL_SEC}s db={DB_PATH}")
    print(f"[poll_aggregated] supported venues: {sorted(SUPPORTED_VENUES)}")
    conn = init_db(DB_PATH)
    n_polls = 0
    while True:
        ts_ms = now_ms()
        try:
            data = fetch_aggregated()
            n_rows, n_symbols, venues_seen = store_snapshot(conn, ts_ms, data)
            log_poll(conn, ts_ms, True, n_symbols, n_rows, venues_seen)
            n_polls += 1
            missing = SUPPORTED_VENUES - venues_seen
            missing_str = f" missing={sorted(missing)}" if missing else ""
            print(f"[{iso_utc(ts_ms)}] poll #{n_polls} ok: {n_symbols} symbols, "
                  f"{n_rows} rows, venues={len(venues_seen)}{missing_str}",
                  flush=True)
        except Exception as e:
            log_poll(conn, ts_ms, False, 0, 0, set(), str(e))
            print(f"[{iso_utc(ts_ms)}] poll FAILED: {e}", file=sys.stderr, flush=True)
        time.sleep(POLL_INTERVAL_SEC)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n[poll_aggregated] interrupted by user")
        sys.exit(0)
