"""
merge_history.py — combine Pacifica + Hyperliquid + Backpack hourly funding
history into a unified sqlite at data/historical_cross_venue.sqlite. The schema
matches data/cross_venue_funding.sqlite so analyze_persistence.py can run on
either DB without modification.

All timestamps are aligned to the nearest hour boundary so that
{symbol, hour_ms} is the join key across venues.
"""
import json
import os
import sqlite3
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PAC_DIR = os.path.join(ROOT, "data", "history")
HL_DIR = os.path.join(ROOT, "data", "history_hyperliquid")
BP_DIR = os.path.join(ROOT, "data", "history_backpack")
DB_PATH = os.path.join(ROOT, "data", "historical_cross_venue.sqlite")


def align_hour_ms(ts_ms: int) -> int:
    return round(ts_ms / 3600000) * 3600000


def init_db(path: str) -> sqlite3.Connection:
    if os.path.exists(path):
        os.remove(path)
    conn = sqlite3.connect(path, isolation_level=None)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute(
        """
        CREATE TABLE funding_aggregated (
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
        CREATE TABLE poll_runs (
            timestamp_ms INTEGER PRIMARY KEY,
            success INTEGER NOT NULL,
            n_symbols INTEGER NOT NULL,
            n_rows INTEGER NOT NULL,
            venues_seen TEXT NOT NULL,
            error TEXT
        )
        """
    )
    conn.execute("CREATE INDEX idx_funding_symbol_ts ON funding_aggregated (symbol, timestamp_ms)")
    conn.execute("CREATE INDEX idx_funding_venue_ts ON funding_aggregated (venue, timestamp_ms)")
    return conn


def load_pacifica(conn: sqlite3.Connection) -> int:
    if not os.path.isdir(PAC_DIR):
        return 0
    n = 0
    for fname in sorted(os.listdir(PAC_DIR)):
        if not fname.endswith(".json"):
            continue
        sym = fname[:-5]
        path = os.path.join(PAC_DIR, fname)
        try:
            rows = json.load(open(path))
        except Exception as e:
            print(f"  pacifica {sym} parse err: {e}", file=sys.stderr)
            continue
        for r in rows:
            ts = align_hour_ms(int(r["created_at"]))
            try:
                rate = float(r["funding_rate"])
            except Exception:
                continue
            conn.execute(
                "INSERT OR IGNORE INTO funding_aggregated VALUES (?,?,?,?)",
                (ts, sym, "pacifica", rate),
            )
            n += 1
    return n


def load_external(conn: sqlite3.Connection, dirpath: str, venue: str) -> int:
    if not os.path.isdir(dirpath):
        return 0
    n = 0
    for fname in sorted(os.listdir(dirpath)):
        if not fname.endswith(".json"):
            continue
        sym = fname[:-5]
        path = os.path.join(dirpath, fname)
        try:
            rows = json.load(open(path))
        except Exception as e:
            print(f"  {venue} {sym} parse err: {e}", file=sys.stderr)
            continue
        for r in rows:
            ts = align_hour_ms(int(r["time_ms"]))
            try:
                rate = float(r["funding_rate"])
            except Exception:
                continue
            conn.execute(
                "INSERT OR IGNORE INTO funding_aggregated VALUES (?,?,?,?)",
                (ts, sym, venue, rate),
            )
            n += 1
    return n


def synthesize_poll_runs(conn: sqlite3.Connection) -> int:
    """Build poll_runs entries from the distinct timestamps in the merged data so
    that analyze_persistence.py's reliability section makes sense."""
    n = 0
    for (ts,) in conn.execute(
        "SELECT DISTINCT timestamp_ms FROM funding_aggregated ORDER BY timestamp_ms"
    ).fetchall():
        venues_present = [
            v for (v,) in conn.execute(
                "SELECT DISTINCT venue FROM funding_aggregated WHERE timestamp_ms=?", (ts,)
            ).fetchall()
        ]
        sym_count = conn.execute(
            "SELECT COUNT(DISTINCT symbol) FROM funding_aggregated WHERE timestamp_ms=?", (ts,)
        ).fetchone()[0]
        row_count = conn.execute(
            "SELECT COUNT(*) FROM funding_aggregated WHERE timestamp_ms=?", (ts,)
        ).fetchone()[0]
        conn.execute(
            "INSERT OR REPLACE INTO poll_runs VALUES (?,?,?,?,?,?)",
            (ts, 1, sym_count, row_count, ",".join(sorted(venues_present)), None),
        )
        n += 1
    return n


def main():
    print(f"Building {DB_PATH}")
    conn = init_db(DB_PATH)
    pac_rows = load_pacifica(conn)
    print(f"  pacifica:    {pac_rows} rows from {PAC_DIR}")
    hl_rows = load_external(conn, HL_DIR, "hyperliquid")
    print(f"  hyperliquid: {hl_rows} rows from {HL_DIR}")
    bp_rows = load_external(conn, BP_DIR, "backpack")
    print(f"  backpack:    {bp_rows} rows from {BP_DIR}")
    poll_rows = synthesize_poll_runs(conn)
    print(f"  synthesized {poll_rows} poll_runs entries")

    print()
    print("Verification:")
    total = conn.execute("SELECT COUNT(*) FROM funding_aggregated").fetchone()[0]
    print(f"  total rows: {total}")
    print(f"  distinct symbols: {conn.execute('SELECT COUNT(DISTINCT symbol) FROM funding_aggregated').fetchone()[0]}")
    print(f"  distinct hours: {conn.execute('SELECT COUNT(DISTINCT timestamp_ms) FROM funding_aggregated').fetchone()[0]}")
    print()
    print("  rows per venue:")
    for v, c in conn.execute("SELECT venue, COUNT(*) FROM funding_aggregated GROUP BY venue ORDER BY venue").fetchall():
        print(f"    {v:<14} {c:>10}")
    print()
    print("  symbols with all 3 venues:")
    rows = conn.execute(
        """
        SELECT symbol, COUNT(DISTINCT venue) as n_venues
        FROM funding_aggregated
        GROUP BY symbol
        HAVING n_venues >= 3
        ORDER BY symbol
        """
    ).fetchall()
    print(f"    {len(rows)} symbols have data on all 3 venues:")
    print(f"    {[r[0] for r in rows]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
