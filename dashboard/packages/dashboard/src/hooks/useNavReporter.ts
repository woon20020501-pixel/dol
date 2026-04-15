"use client";

import { useEffect, useState } from "react";
import { useBotStatus } from "./useBotStatus";
import type { NavReporterStatus } from "../../../../shared/types/bot-api";

const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";

const DEMO_REPORTER: NavReporterStatus = {
  status: "live",
  operatorAddress: "0x10185b89dc3F5A8341e0d8c8731B9b6D749E5a83",
  lastReportTimestamp: Math.floor(Date.now() / 1000) - 5 * 60,
  lastReportNav: 28_450,
  lastReportTxHash:
    "0xa1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef123456",
  nextReportInSec: 4 * 60 + 32,
};

export type UseNavReporterResult = {
  status: NavReporterStatus["status"];
  operatorAddress: string | null;
  lastReportTimestamp: number | null;
  lastReportNav: number | null;
  lastReportTxHash: string | null;
  nextReportInSec: number | null;
  errorMessage: string | null;
  isLoading: boolean;
  isAvailable: boolean; // true if the bot has surfaced reporter info at all
};

/**
 * Reads NAV reporter status from the bot's /status endpoint.
 *
 * Graceful degradation:
 *  - bot offline / status not yet loaded → isLoading or status:"never"
 *  - bot online but no navReporter field  → status:"never", isAvailable:false
 *  - demo mode → returns DEMO_REPORTER with a live countdown
 */
/**
 * Map the bot's canonical reporter shape into the dashboard's expected
 * shape. As of Phase 8, the bot emits:
 *   { operatorAddress, mode, lastReportTimestamp,
 *     lastReportNavUsdc, lastReportTxHash, nextReportInSec, intervalSec }
 * while this hook was originally designed against a provisional
 * shape that used `status` and `lastReportNav`. Accept both so the
 * provisional type stays valid and the live bot lights up.
 */
function readReporter(raw: unknown): NavReporterStatus | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;

  const rawStatus = (r.status ?? r.mode) as string | undefined;
  // Phase 9a decoupled NAV reporter mode from the master dryRun flag.
  // If a tx hash exists the reporter has submitted on-chain → treat as live
  // regardless of what the mode field says.
  const hasOnChainReport = typeof r.lastReportTxHash === "string" && r.lastReportTxHash.length > 0;
  const status: NavReporterStatus["status"] = hasOnChainReport
    ? "live"
    : rawStatus === "live" ||
        rawStatus === "dry-run" ||
        rawStatus === "error" ||
        rawStatus === "never"
      ? rawStatus
      : rawStatus
        ? "never"
        : "never";

  const lastReportNav =
    typeof r.lastReportNav === "number"
      ? r.lastReportNav
      : typeof r.lastReportNavUsdc === "number"
        ? r.lastReportNavUsdc
        : null;

  return {
    status,
    operatorAddress: (r.operatorAddress as string | null | undefined) ?? null,
    lastReportTimestamp:
      typeof r.lastReportTimestamp === "number" ? r.lastReportTimestamp : null,
    lastReportNav,
    lastReportTxHash: (r.lastReportTxHash as string | null | undefined) ?? null,
    nextReportInSec:
      typeof r.nextReportInSec === "number" ? r.nextReportInSec : null,
    errorMessage: (r.errorMessage as string | null | undefined) ?? null,
  };
}

export function useNavReporter(): UseNavReporterResult {
  const botStatus = useBotStatus();
  const reporter = readReporter(botStatus.data?.navReporter);

  // Local countdown so the "next in 4m 32s" ticks visually even
  // between bot polls. Re-seeds whenever the upstream value changes.
  const seed = isDemoMode
    ? DEMO_REPORTER.nextReportInSec
    : reporter?.nextReportInSec ?? null;
  const [countdown, setCountdown] = useState<number | null>(seed);

  useEffect(() => {
    setCountdown(seed);
  }, [seed]);

  useEffect(() => {
    if (countdown === null) return;
    const id = setInterval(() => {
      setCountdown((c) => (c !== null && c > 0 ? c - 1 : c));
    }, 1000);
    return () => clearInterval(id);
  }, [countdown !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  if (isDemoMode) {
    return {
      status: DEMO_REPORTER.status,
      operatorAddress: DEMO_REPORTER.operatorAddress,
      lastReportTimestamp: DEMO_REPORTER.lastReportTimestamp,
      lastReportNav: DEMO_REPORTER.lastReportNav,
      lastReportTxHash: DEMO_REPORTER.lastReportTxHash,
      nextReportInSec: countdown,
      errorMessage: null,
      isLoading: false,
      isAvailable: true,
    };
  }

  if (botStatus.isLoading) {
    return {
      status: "never",
      operatorAddress: null,
      lastReportTimestamp: null,
      lastReportNav: null,
      lastReportTxHash: null,
      nextReportInSec: null,
      errorMessage: null,
      isLoading: true,
      isAvailable: false,
    };
  }

  if (!reporter) {
    return {
      status: "never",
      operatorAddress: null,
      lastReportTimestamp: null,
      lastReportNav: null,
      lastReportTxHash: null,
      nextReportInSec: null,
      errorMessage: null,
      isLoading: false,
      isAvailable: false,
    };
  }

  return {
    status: reporter.status,
    operatorAddress: reporter.operatorAddress,
    lastReportTimestamp: reporter.lastReportTimestamp,
    lastReportNav: reporter.lastReportNav,
    lastReportTxHash: reporter.lastReportTxHash,
    nextReportInSec: countdown,
    errorMessage: reporter.errorMessage ?? null,
    isLoading: false,
    isAvailable: true,
  };
}
