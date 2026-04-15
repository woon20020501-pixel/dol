"use client";

import { useQuery } from "@tanstack/react-query";
import { useRef, useEffect, useState } from "react";
import { getHealth } from "@/lib/botApi";
import { DEMO_HEALTH } from "@/lib/demoData";
import type { HealthResponse } from "../../../../shared/types/bot-api";

const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";

/** Milliseconds of consecutive failures before declaring offline */
const OFFLINE_THRESHOLD_MS = 15_000;

export function useBotHealth() {
  const firstFailureRef = useRef<number | null>(null);
  const [isOffline, setIsOffline] = useState(false);

  const query = useQuery<HealthResponse>({
    queryKey: ["bot-health"],
    queryFn: getHealth,
    refetchInterval: 10_000,
    retry: 1,
    staleTime: 15_000,
    enabled: !isDemoMode,
  });

  useEffect(() => {
    if (isDemoMode) return;

    if (query.isError) {
      if (firstFailureRef.current === null) {
        firstFailureRef.current = Date.now();
      }
      const elapsed = Date.now() - firstFailureRef.current;
      if (elapsed >= OFFLINE_THRESHOLD_MS) {
        setIsOffline(true);
      }
    } else if (query.data) {
      firstFailureRef.current = null;
      setIsOffline(false);
    }
  }, [query.isError, query.data]);

  if (isDemoMode) {
    return {
      data: DEMO_HEALTH as HealthResponse,
      isLoading: false,
      isError: false,
      isOffline: false,
    };
  }

  return {
    data: query.data ?? null,
    isLoading: query.isLoading,
    isError: query.isError,
    isOffline,
  };
}
