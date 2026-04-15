import { ImageResponse } from "next/og";

/**
 * Dynamic Open Graph image for the landing page.
 *
 * Next.js 14 App Router picks up `opengraph-image.tsx` automatically
 * and wires it as the page's `og:image` meta tag. ImageResponse
 * compiles a JSX tree into a PNG at build time — no runtime rendering,
 * no new dependencies (the ImageResponse primitive ships with Next).
 *
 * Design: black background, oversized "A dollar that grows itself."
 * headline in the Inter stack, 1 Dol = 1 USDC byline, Apple-minimal
 * tracking. 1200x630 is the canonical OG size used by every social
 * platform.
 */

export const runtime = "edge";
export const alt = "Dol — A dollar that grows itself.";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function OgImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          background: "#000",
          color: "#fff",
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          padding: "72px",
          fontFamily: "Inter, system-ui, sans-serif",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            fontSize: 28,
            letterSpacing: "-0.01em",
            color: "rgba(255,255,255,0.5)",
          }}
        >
          DOL
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
          <div
            style={{
              fontSize: 96,
              fontWeight: 700,
              letterSpacing: "-0.04em",
              lineHeight: 1.02,
              maxWidth: "1000px",
            }}
          >
            A dollar that grows itself.
          </div>
          <div
            style={{
              fontSize: 32,
              color: "rgba(255,255,255,0.6)",
              letterSpacing: "-0.01em",
            }}
          >
            1 Dol = 1 USDC. Always backed, always redeemable.
          </div>
        </div>

        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            fontSize: 22,
            color: "rgba(255,255,255,0.35)",
            letterSpacing: "0.04em",
          }}
        >
          <div style={{ display: "flex" }}>Up to 7.5% a year</div>
          <div style={{ display: "flex" }}>dol.app</div>
        </div>
      </div>
    ),
    {
      ...size,
    },
  );
}
