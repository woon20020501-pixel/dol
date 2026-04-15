import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/siteUrl";

/**
 * Static sitemap.xml — Next 14 emits this at build time from the
 * returned array. Lists every public marketing/reference route so
 * search engines have a single index to crawl.
 *
 * User-specific routes (/my-dol, /deposit, /dashboard) are
 * deliberately excluded — they're gated behind wallet auth and
 * their indexable content is zero for an unauthenticated crawler.
 *
 * Keep this list in sync with src/content/docs/ filenames.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const base = SITE_URL;
  const now = new Date();

  const staticRoutes = [
    "",
    "/faq",
    "/unavailable",
    "/legal/terms",
    "/legal/privacy",
    "/legal/risk",
    "/docs",
    "/docs/getting-started/what-is-dol",
    "/docs/getting-started/how-to-buy",
    "/docs/getting-started/supported-countries",
    "/docs/how-it-works",
    "/docs/trust/on-chain",
    "/docs/trust/risks",
    "/docs/faq",
    "/docs/more/support",
    "/docs/more/legal",
  ];

  return staticRoutes.map((path) => ({
    url: `${base}${path}`,
    lastModified: now,
    changeFrequency: path === "" ? "weekly" : "monthly",
    priority: path === "" ? 1.0 : 0.7,
  }));
}
