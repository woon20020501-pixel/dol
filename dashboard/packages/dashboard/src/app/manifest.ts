import type { MetadataRoute } from "next";

/**
 * PWA manifest — Next 14 emits it as /manifest.webmanifest.
 *
 * Declaring it makes the site installable as a PWA on Android and
 * desktop Chrome/Edge and gives the browser a consistent icon + name
 * for "Add to Home Screen". Dol Phase 1 isn't a full PWA (no service
 * worker, no offline mode), but the manifest itself is free meta and
 * bumps Lighthouse's PWA installability checks.
 */
export default function manifest(): MetadataRoute.Manifest {
  return {
    name: "Dol",
    short_name: "Dol",
    description:
      "Hold a Dol. Watch it grow. 1 Dol = 1 USDC, always backed, always redeemable.",
    start_url: "/",
    display: "standalone",
    background_color: "#000000",
    theme_color: "#000000",
    orientation: "portrait",
    icons: [
      {
        src: "/icon",
        sizes: "256x256",
        type: "image/png",
        purpose: "any",
      },
      {
        src: "/apple-icon",
        sizes: "180x180",
        type: "image/png",
        purpose: "maskable",
      },
    ],
    categories: ["finance"],
  };
}
