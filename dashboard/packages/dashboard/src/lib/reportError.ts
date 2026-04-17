/**
 * Error reporting primitive.
 *
 * Why this exists:
 *   - app/error.tsx, app/global-error.tsx, and app/dashboard/error.tsx
 *     each console.error() the crash. Fine for dev, but it means
 *     nothing in production — no history, no alerts, no correlation.
 *   - A real production app would pipe these to Sentry / Datadog /
 *     custom backend. Installing Sentry without a DSN adds ~50 KB of
 *     bundle for zero behavior change, so we DON'T do that.
 *
 * What this does:
 *   - Structured extraction of error context (message, stack, digest,
 *     route, user agent, timestamp).
 *   - Always logs to console in development (unchanged behavior).
 *   - If NEXT_PUBLIC_ERROR_REPORT_URL is set, POSTs the structured
 *     payload to that URL as JSON. Fire-and-forget — never throws,
 *     never blocks the caller.
 *   - Single function any error boundary can call, same semantics
 *     across the app.
 *
 * Enabling production error collection later is a single Vercel
 * env-var change — no code edit needed.
 */

export type ErrorContext = {
  route?: string;
  source?: "root" | "global" | "dashboard" | "manual";
  extra?: Record<string, unknown>;
};

export type ErrorReport = {
  message: string;
  name: string;
  stack?: string;
  digest?: string;
  route?: string;
  source?: string;
  userAgent?: string;
  timestampMs: number;
  extra?: Record<string, unknown>;
};

export function reportError(
  error: Error & { digest?: string },
  ctx: ErrorContext = {},
): void {
  const report: ErrorReport = {
    message: error.message,
    name: error.name,
    stack: error.stack,
    digest: error.digest,
    route:
      ctx.route ??
      (typeof window !== "undefined" ? window.location.pathname : undefined),
    source: ctx.source ?? "manual",
    userAgent:
      typeof navigator !== "undefined" ? navigator.userAgent : undefined,
    timestampMs: Date.now(),
    extra: ctx.extra,
  };

  // Always log in dev. `log.error` would also work but we want the
  // stack visible so we use direct console.error here, matching what
  // each error.tsx already does.
  if (process.env.NODE_ENV !== "production") {
    // eslint-disable-next-line no-console
    console.error("[Dol] reportError:", report);
  }

  // Optional production sink. Skip cleanly if not configured —
  // no-op means zero overhead when the env var is unset.
  const endpoint = process.env.NEXT_PUBLIC_ERROR_REPORT_URL;
  if (
    endpoint &&
    typeof window !== "undefined" &&
    typeof fetch === "function"
  ) {
    // Fire-and-forget. Use keepalive so the request survives even
    // if the page is unloading due to the crash.
    try {
      void fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(report),
        keepalive: true,
      }).catch(() => {
        /* swallow — we can't report a reporter failure */
      });
    } catch {
      /* ignore */
    }
  }
}
