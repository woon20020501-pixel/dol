"use client";

import { AlertTriangle } from "lucide-react";

export function OfflineBanner({
  visible,
  message,
}: {
  visible: boolean;
  message?: string;
}) {
  if (!visible) return null;

  return (
    <div
      role="alert"
      className="flex items-center gap-2 rounded-xl border border-carry-amber/30 bg-carry-amber/5 px-4 py-2.5 text-[13px] text-carry-amber"
    >
      <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />
      <span>
        {message ??
          "Bot is offline. Showing last known data. Some values may be stale."}
      </span>
    </div>
  );
}
