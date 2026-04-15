import type { Metadata, Viewport } from "next";
import { Inter, JetBrains_Mono } from "next/font/google";
import { Toaster } from "@/components/ui/sonner";
import {
  VisitGateModal,
  CommandPalette,
} from "@/components/LazyClientComponents";
import { Providers } from "./providers";
import { SITE_URL } from "@/lib/siteUrl";
import "./globals.css";

// Variable font, display swap to avoid FOIT.
// adjustFontFallback: true auto-generates size-adjust metrics so the
// fallback font renders at the same dimensions as Inter while loading,
// preventing CLS on hydration.
const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  display: "swap",
  adjustFontFallback: true,
  preload: true,
});

const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  variable: "--font-jetbrains",
  display: "swap",
  preload: false, // not used on critical path
});

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: {
    default: "Dol — A dollar that grows itself.",
    template: "%s · Dol",
  },
  description:
    "Hold a Dol. Watch it grow — up to 7.5% a year. 1 Dol is always backed 1:1 by USDC. Cash out anytime.",
  keywords: ["Dol", "crypto savings", "USDC", "interest-bearing", "stablecoin"],
  authors: [{ name: "Dol" }],
  openGraph: {
    title: "Dol — A dollar that grows itself.",
    description:
      "Hold a Dol. Watch it grow — up to 7.5% a year. Cash out anytime.",
    images: ["/images/dol.png"],
    type: "website",
    siteName: "Dol",
  },
  twitter: {
    card: "summary_large_image",
    title: "Dol — A dollar that grows itself.",
    description:
      "Hold a Dol. Watch it grow — up to 7.5% a year. Cash out anytime.",
    images: ["/images/dol.png"],
  },
  robots: {
    index: true,
    follow: true,
    googleBot: {
      index: true,
      follow: true,
      "max-image-preview": "large",
      "max-snippet": -1,
      "max-video-preview": -1,
    },
  },
  // icons is intentionally omitted — src/app/icon.tsx is picked up
  // automatically by Next 14's metadata API.
};

export const viewport: Viewport = {
  themeColor: "#000000",
  colorScheme: "dark",
  width: "device-width",
  initialScale: 1,
  maximumScale: 5,
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${inter.variable} ${jetbrainsMono.variable}`}
    >
      <body className="antialiased min-h-screen bg-black text-white">
        {/* Skip link — sr-only until focused, visible on tab */}
        <a
          href="#main"
          className="sr-only focus:not-sr-only focus:fixed focus:top-4 focus:left-4 focus:z-[100] focus:rounded-full focus:bg-white focus:px-5 focus:py-2 focus:text-sm focus:font-medium focus:text-black focus:shadow-2xl focus:outline-none"
        >
          Skip to content
        </a>
        {/*
          Noscript fallback — when JavaScript is disabled, the React
          app won't hydrate and the user sees a blank black screen.
          Show a static "please enable JS" card with links to the
          legal + docs routes, which are still reachable statically.
        */}
        <noscript>
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              minHeight: "100vh",
              padding: "24px",
              textAlign: "center",
              fontFamily: "system-ui, sans-serif",
              color: "white",
            }}
          >
            <h1
              style={{
                fontSize: "32px",
                fontWeight: 700,
                letterSpacing: "-0.02em",
                margin: 0,
              }}
            >
              Dol needs JavaScript.
            </h1>
            <p
              style={{
                marginTop: "12px",
                fontSize: "15px",
                color: "rgba(255,255,255,0.6)",
                maxWidth: "420px",
              }}
            >
              Please enable JavaScript in your browser to use Dol. You can
              still read our{" "}
              <a
                href="/legal/terms"
                style={{ color: "white", textDecoration: "underline" }}
              >
                terms
              </a>
              ,{" "}
              <a
                href="/legal/privacy"
                style={{ color: "white", textDecoration: "underline" }}
              >
                privacy
              </a>
              , and{" "}
              <a
                href="/legal/risk"
                style={{ color: "white", textDecoration: "underline" }}
              >
                risk disclosure
              </a>{" "}
              without scripts.
            </p>
          </div>
        </noscript>
        <Providers>
          <div id="main">{children}</div>
          <VisitGateModal />
          <CommandPalette />
          <Toaster
            theme="dark"
            position="bottom-right"
            visibleToasts={3}
            closeButton
            toastOptions={{
              style: {
                background: "#0a0a0a",
                border: "1px solid rgba(255,255,255,0.08)",
                color: "rgba(255,255,255,0.92)",
              },
            }}
          />
        </Providers>
      </body>
    </html>
  );
}
