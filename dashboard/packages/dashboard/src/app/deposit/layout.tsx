import type { Metadata } from "next";

/**
 * Server layout wrapper so /deposit can carry its own page metadata
 * (the page itself is a client component). Title + OG copy is tuned
 * for a user arriving at the deposit page from a share link or CTA,
 * distinct from the hero landing.
 */
// openGraph.images / twitter.images intentionally OMITTED so this
// route inherits the root's dynamic opengraph-image.tsx (lean 1200x630
// PNG) instead of the 4.3 MB static /images/dol.png which was making
// Twitter/Slack/Discord previews slow to render.
export const metadata: Metadata = {
  title: "Buy your first Dol",
  description:
    "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
  openGraph: {
    title: "Buy your first Dol · Dol",
    description:
      "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
    type: "website",
    siteName: "Dol",
  },
  twitter: {
    card: "summary_large_image",
    title: "Buy your first Dol · Dol",
    description:
      "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
  },
  robots: {
    index: false,
    follow: false,
  },
};

export default function DepositLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
