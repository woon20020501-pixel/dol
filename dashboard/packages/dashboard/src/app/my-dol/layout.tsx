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
export const metadata: Metadata = {
  title: "Your Dol",
  description:
    "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
  openGraph: {
    title: "Your Dol · Dol",
    description:
      "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
    images: ["/images/dol.png"],
    type: "website",
    siteName: "Dol",
  },
  twitter: {
    card: "summary_large_image",
    title: "Your Dol · Dol",
    description:
      "Watch your Dol grow in real time. Cash out anytime — instant or scheduled.",
    images: ["/images/dol.png"],
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
