import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/siteUrl";

/**
 * robots.txt — generated at build time by Next 14's metadata API.
 *
 * Allow crawlers on the public marketing surfaces (landing, docs,
 * faq, legal, unavailable) and deny them on user-specific routes
 * (/my-dol, /deposit, /dashboard) where the output is personal or
 * operational. The /api prefix is reserved even though no routes
 * exist there today.
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: ["/", "/docs", "/docs/", "/faq", "/legal", "/legal/", "/unavailable"],
        disallow: ["/my-dol", "/deposit", "/dashboard", "/api/"],
      },
    ],
    sitemap: `${SITE_URL}/sitemap.xml`,
  };
}
