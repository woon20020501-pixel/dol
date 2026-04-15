import bundleAnalyzer from "@next/bundle-analyzer";

const withBundleAnalyzer = bundleAnalyzer({
  enabled: process.env.ANALYZE === "true",
});

/**
 * Security headers applied globally.
 *
 * CSP note: Privy + wagmi need inline scripts + remote wss for websocket
 * RPC; we loosen `script-src` and `connect-src` to accommodate. If we
 * move off Privy, tighten these.
 *
 * Trusted Types: shipped in *report-only* mode. Enforcing
 * `require-trusted-types-for 'script'` would break any library that
 * assigns to `.innerHTML` without wrapping in a policy — Privy + React
 * internals haven't been audited for this yet. Report-only lets us
 * collect violations from real traffic before flipping enforcement.
 *
 * SRI: external fonts are loaded via `next/font/google`, which
 * self-hosts the files at build time, so there are no third-party
 * `<link>` tags to SRI-pin. No external `<script>` tags exist either.
 * If we ever add a third-party widget (analytics, chat), add an
 * `integrity=` attribute at that time.
 */
//
// Line-by-line gap analysis against facebook.com's response headers
// (tcpdumped 2026-04-14). Everything we could reasonably match WITHOUT
// a Privy rewrite is closed below. The two remaining gaps are:
//
//   1. CSP script-src uses `'unsafe-inline' 'unsafe-eval'` instead of
//      nonces. Privy / wagmi / viem require both. Closing this means
//      ripping Privy out of the layout — tracked as a post-launch item.
//   2. We don't run a CSP reporting endpoint, so violations log to the
//      browser console instead of a collection server. Phase 4 concern.
//
const securityHeaders = [
  // ── Framing + clickjacking ────────────────────────────────────────
  {
    key: "X-Frame-Options",
    value: "DENY",
  },
  // ── Main CSP ──────────────────────────────────────────────────────
  {
    key: "Content-Security-Policy",
    value: [
      "default-src 'self'",
      "script-src 'self' 'unsafe-inline' 'unsafe-eval' https://*.privy.io https://*.privy.systems https://challenges.cloudflare.com",
      "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
      "font-src 'self' data: https://fonts.gstatic.com",
      "img-src 'self' data: blob: https:",
      "connect-src 'self' https: wss: http://localhost:* ws://localhost:*",
      "frame-src 'self' https://*.privy.io https://auth.privy.io https://challenges.cloudflare.com",
      "frame-ancestors 'none'",
      "form-action 'self'",
      "base-uri 'self'",
      "object-src 'none'",
      // Gaps closed vs facebook.com — these were missing before.
      "worker-src 'self' blob:",
      "manifest-src 'self'",
      "media-src 'self'",
      "child-src 'self' blob:",
      "block-all-mixed-content",
      "upgrade-insecure-requests",
    ].join("; "),
  },
  // ── Trusted Types report-only ─────────────────────────────────────
  {
    // Trusted Types in report-only mode — see note above. Flip to
    // `Content-Security-Policy` (enforce) once we've verified no
    // Privy/React DOM sinks violate it.
    key: "Content-Security-Policy-Report-Only",
    value: [
      "require-trusted-types-for 'script'",
      "trusted-types default 'allow-duplicates'",
    ].join("; "),
  },
  // ── Browser API permissions — match Facebook's deny-by-default ────
  //
  // FB disables 40+ features at the origin level. We mirror everything
  // that isn't actively needed. `self` is allowed where future Dol
  // features might need it (fullscreen charts, clipboard in wallet
  // copy, WebAuthn for passkey auth) so the toggle is future-proof.
  //
  {
    key: "Permissions-Policy",
    value: [
      "accelerometer=()",
      "ambient-light-sensor=()",
      "attribution-reporting=()",
      "autoplay=()",
      "battery=()",
      "bluetooth=()",
      "browsing-topics=()",
      "camera=()",
      "clipboard-read=(self)",
      "clipboard-write=(self)",
      "compute-pressure=()",
      "cross-origin-isolated=()",
      "display-capture=()",
      "document-domain=()",
      "encrypted-media=()",
      "execution-while-not-rendered=()",
      "execution-while-out-of-viewport=()",
      "fullscreen=(self)",
      "gamepad=()",
      "geolocation=()",
      "gyroscope=()",
      "hid=()",
      "idle-detection=()",
      "interest-cohort=()",
      "keyboard-map=()",
      "local-fonts=()",
      "magnetometer=()",
      "microphone=()",
      "midi=()",
      "navigation-override=()",
      "otp-credentials=()",
      "payment=()",
      "picture-in-picture=()",
      "publickey-credentials-get=(self)",
      "screen-wake-lock=()",
      "serial=()",
      "speaker-selection=()",
      "storage-access=(self)",
      "sync-xhr=()",
      "unload=()",
      "usb=()",
      "web-share=(self)",
      "window-management=()",
      "xr-spatial-tracking=()",
    ].join(", "),
  },
  // ── Referrer policy ───────────────────────────────────────────────
  {
    key: "Referrer-Policy",
    value: "strict-origin-when-cross-origin",
  },
  // ── Content sniff protection ──────────────────────────────────────
  {
    key: "X-Content-Type-Options",
    value: "nosniff",
  },
  // ── Cross-origin isolation (COOP / CORP / OAC) ────────────────────
  //
  // Same-origin COOP severs the opener relationship so a newly opened
  // tab can't `window.opener.location = evil`. CORP same-origin stops
  // other origins from loading our pages as resources (img, iframe,
  // fetch). Origin-Agent-Cluster hard-isolates the JS heap from other
  // same-site documents for Spectre mitigation. These three are the
  // modern isolation trio Facebook uses.
  //
  {
    key: "Cross-Origin-Opener-Policy",
    value: "same-origin",
  },
  {
    key: "Cross-Origin-Resource-Policy",
    value: "same-origin",
  },
  {
    key: "Origin-Agent-Cluster",
    value: "?1",
  },
  // ── Legacy XSS auditor — explicitly disabled ──────────────────────
  //
  // Every modern browser has removed the XSS auditor, but some older
  // user agents still run it. Setting `0` disables it; the vulnerable
  // mode is `1; mode=block`. CSP handles real XSS these days.
  //
  {
    key: "X-XSS-Protection",
    value: "0",
  },
  // ── Performance / DNS ─────────────────────────────────────────────
  {
    key: "X-DNS-Prefetch-Control",
    value: "on",
  },
];

/** @type {import('next').NextConfig} */
const nextConfig = {
  // Tree-shake lucide-react barrel imports — each icon becomes its own module
  // so only the imported icons land in the bundle instead of the whole pack.
  modularizeImports: {
    "lucide-react": {
      transform: "lucide-react/dist/esm/icons/{{kebabCase member}}",
      preventFullImport: true,
    },
  },
  // Next.js's optimizePackageImports does similar work for common libraries
  experimental: {
    optimizePackageImports: ["framer-motion", "lucide-react", "sonner"],
  },
  async headers() {
    return [
      {
        source: "/:path*",
        headers: securityHeaders,
      },
    ];
  },
};

export default withBundleAnalyzer(nextConfig);
