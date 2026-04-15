/**
 * Single source of truth for "where is this app deployed right now".
 *
 * Priority:
 *   1. NEXT_PUBLIC_SITE_URL      — explicit override (custom domain)
 *   2. VERCEL_PROJECT_PRODUCTION_URL — Vercel's stable production alias
 *   3. VERCEL_URL                — deployment-specific preview URL
 *   4. Hard fallback to the currently-known vercel.app host
 *
 * Used by layout.tsx metadata, robots.ts, sitemap.ts so a Vercel
 * project rename propagates automatically — no code edits required
 * when the project name changes. Add NEXT_PUBLIC_SITE_URL env var
 * only when a custom domain (e.g. dol.app) is pointed at the app.
 */
export const SITE_URL: string =
  process.env.NEXT_PUBLIC_SITE_URL ??
  (process.env.VERCEL_PROJECT_PRODUCTION_URL
    ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
    : process.env.VERCEL_URL
      ? `https://${process.env.VERCEL_URL}`
      : "https://dol-finance.vercel.app");
