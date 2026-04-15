# Pacifica Funding API — Phase 0 Discovery

**Status:** Phase 0 discovery deliverable

## TL;DR

Pacifica exposes a fully **public, key-less REST API** at `https://api.pacifica.fi`. The two endpoints needed for the live strategy layer are `/api/v1/info/prices` (real-time snapshot of all 63 instruments — funding, mark, oracle, OI, volume) and `/api/v1/funding_rate/history?symbol=…` (per-symbol historical funding ticks). **No Playwright is required.** A plain `requests`/`aiohttp` poller is sufficient. Funding cadence is **1 hour**.

## Endpoint inventory

All endpoints are public (no API key, no signature, no cookies). Server is fronted by CloudFront (`X-Amz-Cf-Pop: ICN80-P4`, Seoul edge). Responses are JSON wrapped in `{success, data, error, code}`.

### 1. `GET /api/v1/info` — instrument metadata

Returns 63 instrument descriptors. Per-item fields:

```
symbol, tick_size, min_tick, max_tick, lot_size, max_leverage,
isolated_only, min_order_size, max_order_size,
funding_rate, next_funding_rate, created_at,
instrument_type, base_asset
```

Used for: instrument discovery, leverage limits, min order size sanity-checks. Do **not** poll for funding rates here — `/info/prices` carries the same field plus mark/oracle/OI in one call.

### 2. `GET /api/v1/info/prices` — live market snapshot ★ primary live data source

Returns 63 items. Per-item:

```json
{
  "symbol": "XAU",
  "funding": "0.000015",
  "next_funding": "0.000015",
  "mark": "4767.4",
  "mid": "4767.4",
  "oracle": "4767.860315",
  "open_interest": "201.619",
  "volume_24h": "1641875.9652",
  "yesterday_price": "4710.5",
  "timestamp": 1776139952605
}
```

All numeric fields are **strings** (Pacifica preserves precision; cast with `Decimal` or `float`). `timestamp` is the server snapshot time in ms; it advances every poll, so each call yields a fresh row. This endpoint is the canonical input to the live signal generator.

### 3. `GET /api/v1/funding_rate/history?symbol={SYMBOL}&limit={N}` — historical funding

Per-row:

```json
{
  "oracle_price": "4767.643314",
  "bid_impact_price": "4766.340974",
  "ask_impact_price": "4768.343294",
  "funding_rate": "0.000015",
  "next_funding_rate": "0.000015",
  "created_at": 1776139204506
}
```

Rows are spaced **exactly 1 hour apart** (`1776139204506 − 1776135604506 = 3600000 ms`). Use this to backfill the local sqlite if the poller misses a window, and to validate snapshot data against the live source. `limit` accepts at least 5; full pagination semantics are not yet probed and should be confirmed (look for `before`/`after` query params).

### What does *not* exist (probed and confirmed 404)

`/api/v1/prices`, `/api/v1/funding`, `/api/v1/markets`, `/api/v1/info/markets`, `/api/v1/info/funding`, `/api/v1/info/perps`, `/api/v1/funding_rate_history`, `/api/v1/info/historical_funding`, `/api/v1/orderbook`, `/api/v1/book/{symbol}`, `/api/v1/info/{symbol}`. Orderbook is at `/api/v1/book?symbol=…` (confirmed working, not needed for Phase 0). WebSocket discovery is deferred — REST is sufficient for the planned 5-minute cadence.

## Schema mapping — Pacifica → historical sqlite

Reference `funding_history` table:

```sql
CREATE TABLE funding_history (
  symbol TEXT NOT NULL,
  timestamp_ms INTEGER NOT NULL,
  timestamp_utc TEXT NOT NULL,
  funding_rate REAL NOT NULL,
  next_funding_rate REAL,
  oracle_price REAL,
  bid_impact_price REAL,
  ask_impact_price REAL,
  PRIMARY KEY (symbol, timestamp_ms)
);
```

Mapping from `/api/v1/funding_rate/history`:

| sqlite column      | source                                    | notes                                      |
|--------------------|-------------------------------------------|--------------------------------------------|
| `symbol`           | URL query param                           | not in row body                            |
| `timestamp_ms`     | `created_at`                              | already ms                                 |
| `timestamp_utc`    | `datetime.fromtimestamp(created_at/1000)` | derived                                    |
| `funding_rate`     | `funding_rate`                            | string → float                             |
| `next_funding_rate`| `next_funding_rate`                       | string → float                             |
| `oracle_price`     | `oracle_price`                            | string → float                             |
| `bid_impact_price` | `bid_impact_price`                        | string → float                             |
| `ask_impact_price` | `ask_impact_price`                        | string → float                             |

The mapping is exact. The live poller should reuse `funding_history` byte-for-byte so live and historical datasets can be `UNION ALL`-ed for backtests.

## Symbol coverage check (historical, 90d)

Total rows: **46,174** across 14 symbols:

| basket     | symbol  | rows | first seen           | gap risk                    |
|------------|---------|------|----------------------|-----------------------------|
| stable     | XAU     | 1440 | 2026-02-11           | only ~62 days of history    |
| stable     | PAXG    | 4157 | 2025-10-21           | healthy                     |
| stable     | XAG     | 2470 | 2025-12-30           | healthy                     |
| aggressive | EURUSD  | 1440 | 2026-02-11           | only ~62 days               |
| aggressive | USDJPY  | 2088 | 2026-01-15           | ~89 days                    |
| aggressive | CL      | 2186 | 2026-01-11           | ~93 days                    |
| aggressive | NATGAS  | 1105 | 2026-02-25           | only ~48 days               |

All seven target symbols are present and Pacifica still lists all of them in `/api/v1/info`. **Watch-outs:** XAU, EURUSD, NATGAS each have <70 days of history; backtest windows should respect each symbol's start date or risk extrapolating from a single regime. PLATINUM and SP500 (568 rows each, started 2026-03-19) are also live on Pacifica and are candidates for the aggressive basket later, but are too thin to backtest now.

## Rate limit findings

Rate limit advertised in response headers:

```
ratelimit-policy: "credits";q=1000;w=60
ratelimit:        "credits";r=980;t=53
```

→ **1,000 credits per 60-second sliding window**, ~17 req/s budget. A burst of 12 sequential requests returned 200 on every call with no 429 and no Retry-After. A `/api/v1/info/prices` call once per 5 minutes (12 calls/hour) consumes 0.012 credits/sec — three orders of magnitude under the cap. Even a 5-second cadence would be safe (12 calls/min = 1.2% of budget).

## Cadence findings

Two distinct cadences:

1. **Snapshot freshness** (`/info/prices.timestamp`): updates per request. Two calls 10s apart returned `diff_ms=12002`, i.e. ~real-time. The mark/oracle/OI fields move continuously.
2. **Funding rate tick** (`funding_rate` value): updates **once per hour**, confirmed by two adjacent rows in `/funding_rate/history` being exactly 3,600,000 ms apart. Pacifica is on a 1-hour funding interval.

**Implication:** a signal-generation cadence of every 8 hours is too coarse — funding can flip 8 times before the bot reacts. Polling cadence should be 5 minutes, and signal generation should evaluate every 1 hour (every funding tick).

## Recommended polling approach

- Pure HTTP via `requests` (sync) or `aiohttp` (async) — **no Playwright**.
- Single endpoint: `GET /api/v1/info/prices` every 5 minutes.
- Filter to the tracked symbols.
- Insert into `data/live_funding.sqlite` using the exact `funding_history` schema, with `timestamp_ms` derived from snapshot `timestamp` rather than `created_at` (since `/info/prices` is real-time, not aligned to funding ticks).
- Run a parallel hourly task that calls `/api/v1/funding_rate/history?symbol=…` for each tracked symbol, to capture the canonical funding-tick rows.
- No auth required; no User-Agent gating observed; CloudFront edge latency is ~30 ms.
- Backoff: simple exponential on any non-200, with respect for `ratelimit` header if `r` drops below 100.

## Risks / unknowns

1. **Funding cadence.** Pacifica uses 1 h ticks. Annualized funding from a 0.000015/h rate is ~13.1% APY before mark drift.
2. **Thin history for XAU / EURUSD / NATGAS.** Three of the seven basket symbols have <70 days of data — backtests should respect each symbol's start date or risk extrapolating from a single regime.
3. **No published rate-limit doc.** The 1,000-credit/60s cap is inferred from headers, not documentation. The poller respects the live `ratelimit` header rather than hard-coding the budget.
4. **No WebSocket probing yet.** REST suffices; a WebSocket investigation is deferred.
5. **`/funding_rate/history` pagination unknown.** Tested only with `limit=5`; full pagination semantics (`before` / `after` / `offset`) need confirmation.
6. **Field-type strings.** Every numeric is a string. The poller uses `Decimal` for monetary fields so the Rust port stays consistent.
