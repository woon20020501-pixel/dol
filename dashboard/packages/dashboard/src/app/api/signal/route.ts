import { NextResponse } from "next/server";
import { promises as fs } from "fs";
import path from "path";

/**
 * GET /api/signal — server-side reader for the bot's per-symbol signal JSONs.
 *
 * Reads the latest signal JSON for each known symbol so the dashboard can
 * surface live values for fields that don't appear in nav.jsonl:
 * pair_decision, cycle_lock, fair_value contributing_venues, fsm mode,
 * diagnostics.stubbed_sections, etc.
 *
 * Path layout:
 *   <demo_smoke>/signals/{symbol}/{yyyymmdd}/{ts_ms}.json
 *
 * One file per (symbol, tick). To grab the latest per symbol we walk the
 * symbol directory, find the most recent date subdir, then pick the file
 * with the highest numeric basename.
 */

export const dynamic = "force-dynamic";

const STALE_THRESHOLD_MS = 30_000;

const KNOWN_SYMBOLS = [
  "BTC", "ETH", "SOL", "BNB", "ARB", "AVAX", "SUI", "XAU", "XAG", "PAXG",
];

const DEFAULT_SIGNALS_ROOT = path.resolve(
  process.cwd(),
  "../../../bot-rs/output/demo_smoke/signals",
);

function resolveSignalsRoot(): string {
  return process.env.SIGNALS_ROOT
    ? path.resolve(process.env.SIGNALS_ROOT)
    : DEFAULT_SIGNALS_ROOT;
}

async function readLatestSignal(
  root: string,
  symbol: string,
): Promise<{ data: unknown; mtimeMs: number } | null> {
  const symbolDir = path.join(root, symbol);
  let dateEntries: string[];
  try {
    dateEntries = await fs.readdir(symbolDir);
  } catch {
    return null;
  }
  if (dateEntries.length === 0) return null;
  // Date subdirs are yyyymmdd — lexical sort works.
  dateEntries.sort();
  const latestDateDir = dateEntries[dateEntries.length - 1];

  const dayDir = path.join(symbolDir, latestDateDir);
  let fileEntries: string[];
  try {
    fileEntries = await fs.readdir(dayDir);
  } catch {
    return null;
  }
  const jsonFiles = fileEntries.filter((f) => f.endsWith(".json"));
  if (jsonFiles.length === 0) return null;

  // Filenames are <ts_ms>.json — sort numerically to get latest.
  jsonFiles.sort((a, b) => {
    const ai = Number(a.replace(".json", ""));
    const bi = Number(b.replace(".json", ""));
    return ai - bi;
  });
  const latestFile = jsonFiles[jsonFiles.length - 1];
  const fullPath = path.join(dayDir, latestFile);

  let stat;
  try {
    stat = await fs.stat(fullPath);
  } catch {
    return null;
  }

  let text: string;
  try {
    text = await fs.readFile(fullPath, "utf8");
  } catch {
    return null;
  }

  try {
    return { data: JSON.parse(text), mtimeMs: stat.mtimeMs };
  } catch {
    return null;
  }
}

export async function GET(req: Request) {
  const url = new URL(req.url);
  const symbolParam = url.searchParams.get("symbol");
  const root = resolveSignalsRoot();

  // Verify the root exists. If not, fall back to simulator on the client.
  try {
    await fs.stat(root);
  } catch {
    return NextResponse.json(
      {
        ok: false,
        error: "signals_root_missing",
        detail: `signals root not found at ${root}`,
        fallback_to_simulator: true,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }

  const symbolList = symbolParam
    ? [symbolParam.toUpperCase()]
    : KNOWN_SYMBOLS;

  const signals: Record<string, unknown> = {};
  let maxMtime = 0;

  await Promise.all(
    symbolList.map(async (sym) => {
      const result = await readLatestSignal(root, sym);
      if (result) {
        signals[sym] = result.data;
        if (result.mtimeMs > maxMtime) maxMtime = result.mtimeMs;
      }
    }),
  );

  if (Object.keys(signals).length === 0) {
    return NextResponse.json(
      {
        ok: false,
        error: "signals_empty",
        detail: "no signal JSON found for any symbol",
        fallback_to_simulator: true,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }

  const isStale = Date.now() - maxMtime > STALE_THRESHOLD_MS;

  return NextResponse.json(
    {
      ok: true,
      signals,
      n_symbols: Object.keys(signals).length,
      file_mtime_ms_max: maxMtime,
      is_stale: isStale,
      signals_root: root,
    },
    { status: 200, headers: { "Cache-Control": "no-store" } },
  );
}
