import { ImageResponse } from "next/og";

/**
 * Favicon — Next 14 picks up `app/icon.tsx` and wires it as the
 * site favicon + apple-touch-icon without any metadata.icons hack.
 *
 * Design: the Dol pebble from DolHeroImage, rendered as a minimal
 * flat shape on a black background so the browser tab glyph is
 * legible at 16×16 but also looks sharp at 256×256.
 *
 * ImageResponse is Next's edge-runtime JSX→PNG primitive; no new
 * dependency, no build-time rasterization. The same tool used for
 * opengraph-image.tsx.
 */

export const runtime = "edge";
export const size = { width: 256, height: 256 };
export const contentType = "image/png";

export default function Icon() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          background: "#000",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        {/* Slate pebble — two stacked ellipses + cheek + sparkle,
            matching the homepage DolHeroImage marks. */}
        <svg width="200" height="200" viewBox="0 0 240 204">
          <ellipse cx="120" cy="108" rx="92" ry="80" fill="#94a3b8" />
          <ellipse cx="120" cy="92" rx="92" ry="72" fill="#e2e8f0" />
          <ellipse
            cx="88"
            cy="74"
            rx="22"
            ry="14"
            fill="#f1f5f9"
            opacity="0.7"
          />
          <circle cx="82" cy="68" r="4" fill="#ffffff" />
          <circle cx="82" cy="68" r="1.5" fill="#e2e8f0" />
        </svg>
      </div>
    ),
    {
      ...size,
    },
  );
}
