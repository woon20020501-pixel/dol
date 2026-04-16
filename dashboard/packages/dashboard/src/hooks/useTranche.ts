"use client";

import { useBotStatus } from "./useBotStatus";
import { DOL_APY } from "@/lib/constants";

const SENIOR_TARGET_APY = DOL_APY;
const DEMO_MODE = process.env.NEXT_PUBLIC_DEMO_MODE === "true";

// 30-day average carry score from Excel evidence
// Source: pacifica_vault_trade_log.xlsx, NAV Reports sheet
const DEMO_AVG_CARRY = 0.1274; // 12.74%

export function useTranche() {
  const { data: status } = useBotStatus();

  const rawCarryScore = status?.carryScore?.value ?? 0;
  const isFavorable = rawCarryScore > 0;

  // Demo mode: use 30-day average when live data is unfavorable
  // Dev mode: show raw state (favorable or 0%)
  const effectiveApy = DEMO_MODE
    ? isFavorable
      ? rawCarryScore
      : DEMO_AVG_CARRY
    : isFavorable
      ? rawCarryScore
      : 0;

  const seniorApy = Math.min(effectiveApy, SENIOR_TARGET_APY);
  // Junior gets the residual above the senior cap — simple subtraction
  const juniorApy = Math.max(effectiveApy - seniorApy, 0);

  if (DEMO_MODE && !isFavorable) {
    console.info(
      "[pBond] Demo mode active — showing 30-day average APY. " +
        `Raw carry score: ${(rawCarryScore * 100).toFixed(2)}%`
    );
  }

  return {
    seniorApy,
    juniorApy,
    totalApy: effectiveApy,
    seniorTargetApy: SENIOR_TARGET_APY,
    seniorApyPct: Number((seniorApy * 100).toFixed(1)),
    juniorApyPct: Number((juniorApy * 100).toFixed(1)),
    isFavorable: DEMO_MODE ? true : isFavorable,
    rawCarryScore,
    isDemoMode: DEMO_MODE,
  };
}
