"use client";

import { useEffect } from "react";
import { reportError } from "@/lib/reportError";

/**
 * Last-resort error boundary. app/error.tsx catches crashes inside
 * the root layout's children, but if layout.tsx ITSELF throws
 * (Privy provider init failure, font loader failure, wagmi config
 * error), the root error boundary can't render. Next.js then shows
 * its default "Application error: a client-side exception has
 * occurred" gray screen — ugly and scary for a judge.
 *
 * global-error.tsx is the only component that can rescue that case:
 * it must render its own <html> and <body> because the root layout
 * is down. Keep the markup minimal and dependency-free for this
 * reason — NO Tailwind classes (globals.css may not have loaded),
 * NO external components.
 */
export default function GlobalErrorBoundary({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    reportError(error, { source: "global" });
  }, [error]);

  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          minHeight: "100vh",
          background: "#000",
          color: "#fff",
          fontFamily:
            "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "24px",
        }}
      >
        <div
          role="alert"
          style={{
            maxWidth: "480px",
            textAlign: "center",
          }}
        >
          <div
            style={{
              display: "inline-block",
              width: 64,
              height: 64,
              borderRadius: "50%",
              background: "radial-gradient(circle, #e2e8f0, #94a3b8)",
              marginBottom: 24,
            }}
            aria-hidden="true"
          />
          <h1
            style={{
              fontSize: 28,
              fontWeight: 700,
              letterSpacing: "-0.02em",
              margin: 0,
            }}
          >
            Dol is temporarily unavailable.
          </h1>
          <p
            style={{
              marginTop: 12,
              color: "rgba(255,255,255,0.6)",
              fontSize: 15,
              lineHeight: 1.5,
            }}
          >
            Something went wrong loading the page. Your funds are not
            affected &mdash; they&apos;re held by the on-chain contract, not by
            this website. You can safely reload, or access your Dol
            directly via any Ethereum wallet.
          </p>
          <div style={{ marginTop: 24, display: "flex", gap: 12, justifyContent: "center" }}>
            <button
              type="button"
              onClick={() => reset()}
              style={{
                background: "#fff",
                color: "#000",
                border: "none",
                borderRadius: 9999,
                padding: "10px 22px",
                fontSize: 14,
                fontWeight: 500,
                cursor: "pointer",
              }}
            >
              Reload
            </button>
            <a
              href="/"
              style={{
                color: "rgba(255,255,255,0.7)",
                textDecoration: "none",
                border: "1px solid rgba(255,255,255,0.15)",
                borderRadius: 9999,
                padding: "10px 22px",
                fontSize: 14,
              }}
            >
              Home
            </a>
          </div>
          {error.digest ? (
            <p
              style={{
                marginTop: 20,
                fontSize: 11,
                color: "rgba(255,255,255,0.3)",
                fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
              }}
            >
              digest: {error.digest}
            </p>
          ) : null}
        </div>
      </body>
    </html>
  );
}
