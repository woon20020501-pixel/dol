"use client";

import { Clock } from "lucide-react";

export function StaleIndicator({
  visible,
  ageSeconds,
}: {
  visible: boolean;
  ageSeconds?: number;
}) {
  if (!visible) return null;

  const ageLabel = ageSeconds
    ? ageSeconds >= 60
      ? `${Math.floor(ageSeconds / 60)}m ago`
      : `${ageSeconds}s ago`
    : "stale";

  return (
    <span className="inline-flex items-center gap-1 text-[10px] text-carry-amber" role="status" aria-label={`Data is ${ageLabel}`}>
      <Clock className="h-3 w-3" aria-hidden="true" />
      {ageLabel}
    </span>
  );
}
