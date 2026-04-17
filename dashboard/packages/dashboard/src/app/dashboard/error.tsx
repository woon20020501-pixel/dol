"use client";

import { useEffect } from "react";
import Link from "next/link";
import { reportError } from "@/lib/reportError";

/**
 * Subtree error boundary for /dashboard. The Aurora console does the
 * most real-time work in the app (rAF loop, dual polling, live chart
 * re-renders, Recharts + framer-motion everywhere), so it has the
 * highest surface area for an unexpected throw — a malformed signal
 * JSON, an NaN leaking into a chart domain, a framer transition
 * colliding with an unmount, etc.
 *
 * Scoping the boundary here (vs relying only on the root error.tsx)
 * means an Aurora crash keeps the root layout + header intact and
 * the user can recover with a single button tap instead of a full
 * page reload.
 */
export default function DashboardError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    reportError(error, { source: "dashboard" });
  }, [error]);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-black px-6 py-12 text-white">
      <div
        role="alert"
        className="w-full max-w-md rounded-2xl border border-white/10 bg-white/[0.03] p-8 text-center"
      >
        <div className="mx-auto mb-6 flex h-14 w-14 items-center justify-center rounded-full bg-amber-400/15 text-amber-300">
          <svg
            width="28"
            height="28"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </div>
        <h1
          className="text-2xl font-semibold tracking-[-0.02em]"
          style={{ letterSpacing: "-0.02em" }}
        >
          Operate console hit a snag.
        </h1>
        <p className="mt-3 text-sm leading-relaxed text-white/60">
          The Aurora console crashed while rendering. This is a UI-only
          fault &mdash; on-chain state, your balance, and the bot&apos;s telemetry
          are all unaffected. Tap Reload to recover.
        </p>
        <div className="mt-6 flex items-center justify-center gap-3">
          <button
            type="button"
            onClick={() => reset()}
            className="rounded-full bg-white px-5 py-2 text-sm font-medium text-black transition-colors hover:bg-white/90"
          >
            Reload console
          </button>
          <Link
            href="/"
            className="rounded-full border border-white/15 px-5 py-2 text-sm text-white/70 transition-colors hover:border-white/30 hover:text-white"
          >
            Back to landing
          </Link>
        </div>
        {error.digest ? (
          <p className="mt-5 font-mono text-[10px] text-white/25">
            digest: {error.digest}
          </p>
        ) : null}
      </div>
    </main>
  );
}
