import type { Metadata } from "next";

/**
 * Server-side layout for /my-dol purely so we can export page-level
 * metadata. The page itself is "use client" (needs wagmi / Privy hooks)
 * and therefore cannot export `metadata` directly — Next 14 App Router
 * requires metadata to come from a server component.
 *
 * Marketing title / description is tuned for a signed-in user viewing
 * their portfolio, distinct from the landing hero copy. noindex so we
 * don't expose per-user URLs to search — matches robots.ts.
 */
// Note: openGraph.images and twitter.images are intentionally OMITTED
// so this route inherits the dynamic opengraph-image.tsx at the app
// root (1200x630 PNG, ~50 KB, generated at build). Previously we had
// static "/images/dol.png" references here which pulled the 4.3 MB
// hero artwork — slow social previews on Twitter/Slack/Discord.
export const metadata: Metadata = {
  title: "Your Dol",
  description:
    "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
  openGraph: {
    title: "Your Dol · Dol",
    description:
      "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
    type: "website",
    siteName: "Dol",
  },
  twitter: {
    card: "summary_large_image",
    title: "Your Dol · Dol",
    description:
      "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
  },
  robots: {
    index: false,
    follow: false,
  },
};

export default function MyDolLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
