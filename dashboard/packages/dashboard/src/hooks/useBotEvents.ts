"use client";

import { useQuery } from "@tanstack/react-query";
import { useRef } from "react";
import { getEvents } from "@/lib/botApi";
import {
  generateFundingSpreadData,
  generateCumulativePnlData,
} from "@/lib/demoData";
import type { EventsResponse, SystemEvent } from "../../../../shared/types/bot-api";

const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";

/** Fetch events from the last 24h, filtered for chart data */
export function useBotEvents() {
  const lastGoodRef = useRef<SystemEvent[] | null>(null);

  const since = Math.floor(Date.now() / 1000) - 24 * 3600;

  const query = useQuery<EventsResponse>({
    queryKey: ["bot-events", since],
    queryFn: () => getEvents(since, 1000),
    refetchInterval: 30_000, // refresh every 30s
    retry: 1,
    staleTime: 30_000,
    enabled: !isDemoMode,
  });

  if (query.data?.events && !query.isError) {
    lastGoodRef.current = query.data.events;
  }

  if (isDemoMode) {
    return {
      fundingData: generateFundingSpreadData(),
      pnlData: generateCumulativePnlData(),
      isLoading: false,
      isError: false,
      isStale: false,
    };
  }

  const events = query.data?.events ?? lastGoodRef.current ?? [];
  const observations = events.filter((e) => e.type === "OBSERVATION");

  // Transform OBSERVATION events into chart-friendly format.
  // Bot emits `{venue}_funding.ratePerInterval` as a decimal fraction
  // (0.00012 = 0.012%/interval = 1.2 bps). Convert to bps.
  const fundingData = observations.map((e) => {
    const p = e.payload as Record<string, unknown>;
    const pacificaBps =
      getNestedNumber(p, "pacifica_funding", "ratePerInterval") * 10_000;
    const lighterBps =
      getNestedNumber(p, "lighter_funding", "ratePerInterval") * 10_000;
    return {
      timestamp: Math.floor(e.ts / 1000),
      time: formatTime(e.ts),
      pacifica: Number(pacificaBps.toFixed(3)),
      lighter: Number(lighterBps.toFixed(3)),
      spread: Number((pacificaBps - lighterBps).toFixed(3)),
    };
  });

  // Cumulative PnL: the bot's OBSERVATION events don't currently emit
  // unrealizedPnl (positions carry notional/side/size only). Until a
  // richer event ships, pnlData stays empty and the chart falls back
  // to demo data via the `hasPnlData` check below.
  const pnlData: { timestamp: number; time: string; pnl: number }[] = [];

  const isStale = query.isError && !!lastGoodRef.current;

  // Fall back to demo data if we have no real data AT ALL. A single
  // all-zero funding row would otherwise draw a dead-flat line.
  const hasFundingData =
    fundingData.length > 0 &&
    fundingData.some((d) => d.pacifica !== 0 || d.lighter !== 0);
  const hasPnlData =
    pnlData.length > 0 && pnlData.some((d) => d.pnl !== 0);

  return {
    fundingData: hasFundingData ? fundingData : generateFundingSpreadData(),
    pnlData: hasPnlData ? pnlData : generateCumulativePnlData(),
    isLoading: query.isLoading && !lastGoodRef.current,
    isError: query.isError,
    isStale,
  };
}

// ── Helpers ──

function getNestedNumber(
  obj: Record<string, unknown>,
  ...keys: string[]
): number {
  let current: unknown = obj;
  for (const key of keys) {
    if (current && typeof current === "object") {
      current = (current as Record<string, unknown>)[key];
    } else {
      return 0;
    }
  }
  return typeof current === "number" ? current : 0;
}

function formatTime(tsMs: number): string {
  const d = new Date(tsMs);
  return d.toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
}
