import { NextResponse } from "next/server";
import { promises as fs } from "fs";
import path from "path";

/**
 * GET /api/nav — server-side reader for the bot's `nav.jsonl`.
 *
 * Behaviour:
 *   - Resolves file path from NAV_JSONL_PATH env var or the default
 *     `bot-rs/output/demo_smoke/nav.jsonl` relative to the monorepo root
 *   - Reads the file, splits on newlines, parses each line as JSON,
 *     silently skipping parse errors (handles mid-write partials)
 *   - Supports ?since_ms=<number> to stream only new rows, and
 *     ?tail=<n> to return the last N rows
 *   - Flags `is_stale` when mtime is > 30 s old
 *   - On missing/empty/read error, returns `{ ok: false,
 *     fallback_to_simulator: true }` so the client falls back to the
 *     deterministic simulator path in useAuroraTelemetry
 *
 * The dashboard never writes to `nav.jsonl`. This route is read-only.
 */

export const dynamic = "force-dynamic";

const STALE_THRESHOLD_MS = 30_000;

// Resolve the default relative to the Next.js cwd. In the monorepo layout
// the dashboard runs from `dashboard/packages/dashboard`, so walking up
// three levels lands at the repo root where `bot-rs/` is a sibling folder.
const DEFAULT_NAV_PATH = path.resolve(
  process.cwd(),
  "../../../bot-rs/output/demo_smoke/nav.jsonl",
);

function resolveNavPath(): string {
  return process.env.NAV_JSONL_PATH
    ? path.resolve(process.env.NAV_JSONL_PATH)
    : DEFAULT_NAV_PATH;
}

export async function GET(req: Request) {
  const url = new URL(req.url);
  const sinceMsParam = url.searchParams.get("since_ms");
  const tailParam = url.searchParams.get("tail");
  const navPath = resolveNavPath();

  let stat;
  try {
    stat = await fs.stat(navPath);
  } catch {
    return NextResponse.json(
      {
        ok: false,
        error: "nav_jsonl_missing",
        detail: `nav.jsonl not found at ${navPath}`,
        fallback_to_simulator: true,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }

  let text: string;
  try {
    text = await fs.readFile(navPath, "utf8");
  } catch (e) {
    return NextResponse.json(
      {
        ok: false,
        error: "nav_jsonl_read_error",
        detail: String(e),
        fallback_to_simulator: true,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }

  const lines = text.split("\n").filter((l) => l.trim().length > 0);
  if (lines.length === 0) {
    return NextResponse.json(
      {
        ok: false,
        error: "nav_jsonl_empty",
        fallback_to_simulator: true,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }

  // Parse. Silently skip malformed lines (mid-write partials happen).
  let rows: Record<string, unknown>[] = [];
  for (const line of lines) {
    try {
      const parsed = JSON.parse(line);
      if (parsed && typeof parsed === "object") {
        rows.push(parsed as Record<string, unknown>);
      }
    } catch {
      // Drop the partial/garbage line and keep going.
    }
  }

  if (sinceMsParam) {
    const since = Number(sinceMsParam);
    if (Number.isFinite(since)) {
      rows = rows.filter(
        (r) => typeof r.ts_ms === "number" && (r.ts_ms as number) > since,
      );
    }
  }

  if (tailParam) {
    const n = Number(tailParam);
    if (Number.isFinite(n) && n > 0) {
      rows = rows.slice(-Math.floor(n));
    }
  }

  const latestTs = rows.reduce<number>((acc, r) => {
    const t = typeof r.ts_ms === "number" ? (r.ts_ms as number) : 0;
    return t > acc ? t : acc;
  }, 0);

  const fileMtimeMs = stat.mtimeMs;
  const nowMs = Date.now();
  const isStale = nowMs - fileMtimeMs > STALE_THRESHOLD_MS;

  return NextResponse.json(
    {
      ok: true,
      rows,
      file_path: navPath,
      file_mtime_ms: fileMtimeMs,
      file_size_bytes: stat.size,
      n_rows: rows.length,
      latest_symbol_ts_ms: latestTs,
      is_stale: isStale,
      is_demo_mode: false,
    },
    { status: 200, headers: { "Cache-Control": "no-store" } },
  );
}
