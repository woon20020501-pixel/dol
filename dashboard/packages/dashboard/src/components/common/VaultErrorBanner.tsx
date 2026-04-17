"use client";

import { AlertTriangle } from "lucide-react";
import { translateError, type ErrorCategory } from "@/lib/errors";

/**
 * Surfaces a failed vault contract read (RPC outage, wrong chain,
 * malformed contract state). Renders only when isError is true so
 * the happy path stays visually clean.
 *
 * The message is category-aware via translateError:
 *   - network_glitch (RPC outage, timeout, fetch failed): transient,
 *     auto-retries — message emphasises "will keep retrying".
 *   - wrong_network: the reader is on the wrong chain — tells them.
 *   - contract_revert: vault read itself reverted (unusual for read
 *     calls, but possible on chain state corruption) — factual.
 *   - unknown / any other: generic "read failed" fallback.
 *
 * Previous generic copy mislabelled every failure as "contract read
 * failed", which masked user-actionable cases like wrong network.
 */

type BannerCopy = {
  title: string;
  description: string;
};

function copyFor(category: ErrorCategory): BannerCopy {
  switch (category) {
    case "network_glitch":
      return {
        title: "Connection hiccup",
        description:
          "Couldn't reach the Base Sepolia RPC. Numbers may be stale — the dashboard retries automatically every 10s.",
      };
    case "wrong_network":
      return {
        title: "Wrong network",
        description:
          "Your wallet is on a different chain. Switch to Base Sepolia to see live vault data.",
      };
    case "contract_revert":
      return {
        title: "Vault read reverted",
        description:
          "The contract rejected the read call. Check the deployment is complete and the ABI matches.",
      };
    default:
      return {
        title: "Vault read failed",
        description:
          "Displayed numbers may be stale. The dashboard will keep retrying automatically.",
      };
  }
}

export function VaultErrorBanner({
  isError,
  error,
  onRetry,
}: {
  isError: boolean;
  error?: unknown;
  onRetry?: () => void;
}) {
  if (!isError) return null;

  const category = error ? translateError(error).category : "unknown";
  const { title, description } = copyFor(category);

  return (
    <div
      role="alert"
      className="mb-4 flex items-start gap-2 rounded-xl border border-red-500/30 bg-red-500/5 px-4 py-2.5 text-[13px] text-red-400"
    >
      <AlertTriangle
        className="mt-0.5 h-4 w-4 shrink-0"
        aria-hidden="true"
      />
      <div className="flex-1">
        <p className="font-medium">{title}</p>
        <p className="mt-0.5 text-red-300/80">{description}</p>
      </div>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="shrink-0 self-center rounded-md border border-red-500/40 px-2.5 py-1 text-[12px] font-medium text-red-300 transition-colors hover:bg-red-500/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-400 focus-visible:ring-offset-2 focus-visible:ring-offset-background"
          aria-label="Retry vault contract read"
        >
          Retry
        </button>
      )}
    </div>
  );
}
