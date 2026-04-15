import type { Metadata } from "next";

/**
 * Server layout wrapper so /deposit can carry its own page metadata
 * (the page itself is a client component). Title + OG copy is tuned
 * for a user arriving at the deposit page from a share link or CTA,
 * distinct from the hero landing.
 */
export const metadata: Metadata = {
  title: "Buy your first Dol",
  description:
    "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
  openGraph: {
    title: "Buy your first Dol · Dol",
    description:
      "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
    images: ["/images/dol.png"],
    type: "website",
    siteName: "Dol",
  },
  twitter: {
    card: "summary_large_image",
    title: "Buy your first Dol · Dol",
    description:
      "Get your first Dol in under a minute. 1 Dol = 1 USDC, backed on-chain, redeemable anytime.",
    images: ["/images/dol.png"],
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
