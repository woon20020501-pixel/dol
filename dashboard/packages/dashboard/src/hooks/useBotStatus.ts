"use client";

import { useQuery } from "@tanstack/react-query";
import { useRef } from "react";
import { getStatus } from "@/lib/botApi";
import { DEMO_STATUS } from "@/lib/demoData";
import type { StatusResponse } from "../../../../shared/types/bot-api";

const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";

/**
 * Normalizes the bot's canonical /status shape into the form the
 * dashboard's components expect. The bot is canonical as of Phase 8,
 * but the dashboard's internal convention (set during Phase 1 mock
 * data) differs on two fields:
 *
 *  - fundingRates.*.apyEquivalent: bot emits decimal-fraction (0.1314
 *    = 13.14%); dashboard treats as percent. Multiply by 100.
 *  - positions.*.side: bot emits lowercase "short"/"long"; dashboard
 *    expects uppercase. Upper-case here so the colorizer and label
 *    both work without per-site patches.
 *
 * Single normalization point keeps the rest of the dashboard unaware
 * of the bot shape.
 */
function normalizeStatus(raw: StatusResponse | undefined): StatusResponse | undefined {
  if (!raw) return raw;

  const normSide = (s: string | undefined): "LONG" | "SHORT" =>
    (String(s ?? "").toUpperCase() as "LONG" | "SHORT");

  const pac = raw.positions?.pacifica;
  const hed = raw.positions?.hedge;

  return {
    ...raw,
    fundingRates: raw.fundingRates
      ? {
          pacifica: {
            ...raw.fundingRates.pacifica,
            apyEquivalent: raw.fundingRates.pacifica.apyEquivalent * 100,
          },
          lighter: {
            ...raw.fundingRates.lighter,
            apyEquivalent: raw.fundingRates.lighter.apyEquivalent * 100,
          },
        }
      : raw.fundingRates,
    positions: {
      pacifica: pac ? { ...pac, side: normSide(pac.side) } : null,
      hedge: hed ? { ...hed, side: normSide(hed.side) } : null,
    },
  };
}

export function useBotStatus() {
  const lastGoodRef = useRef<StatusResponse | null>(null);

  const query = useQuery<StatusResponse>({
    queryKey: ["bot-status"],
    queryFn: getStatus,
    refetchInterval: 3_000,
    retry: 2,
    staleTime: 6_000,
    enabled: !isDemoMode,
  });

  const normalized = normalizeStatus(query.data);

  // Track last good data for graceful degradation
  if (normalized && !query.isError) {
    lastGoodRef.current = normalized;
  }

  if (isDemoMode) {
    return {
      data: DEMO_STATUS as unknown as StatusResponse,
      isLoading: false,
      isError: false,
      isStale: false,
      isOffline: false,
      lastGoodData: DEMO_STATUS as unknown as StatusResponse,
      staleAgeSeconds: 0,
    };
  }

  const lastGoodData = lastGoodRef.current ?? (DEMO_STATUS as unknown as StatusResponse);
  const isOffline = query.isError && !lastGoodRef.current;
  const isStale = query.isError && !!lastGoodRef.current;

  // Calculate stale age
  const staleAgeSeconds = isStale && lastGoodData?.nav?.timestamp
    ? Math.floor(Date.now() / 1000 - lastGoodData.nav.timestamp)
    : 0;

  return {
    data: normalized ?? lastGoodData,
    isLoading: query.isLoading && !lastGoodRef.current,
    isError: query.isError,
    isStale,
    isOffline,
    lastGoodData,
    staleAgeSeconds,
  };
}
