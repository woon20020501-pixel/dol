"use client";

import { useEffect } from "react";
import Link from "next/link";
import DolHeroImage from "@/components/DolHeroImage";

/**
 * Global error boundary. Catches unexpected runtime errors in the
 * consumer surfaces (/, /deposit, /my-dol) and shows a friendly
 * recovery screen instead of the raw Next.js error overlay.
 */
export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Surface to console for debugging but don't show to user
    // eslint-disable-next-line no-console
    console.error("[Dol] Unhandled error:", error);
  }, [error]);

  return (
    <main className="min-h-screen bg-black text-white flex flex-col items-center justify-center px-6 py-12">
      <DolHeroImage size={200} />

      <h1
        className="mt-12 text-4xl md:text-5xl font-bold text-white text-center leading-[1.05]"
        style={{ letterSpacing: "-0.04em" }}
      >
        Something went wrong.
      </h1>

      <p className="mt-4 max-w-md text-center text-[15px] text-white/50">
        The page ran into an unexpected hiccup. Your funds are not affected —
        only this view crashed. Please try again.
      </p>

      <div className="mt-10 flex flex-col sm:flex-row items-center gap-3">
        <button
          onClick={reset}
          className="rounded-full bg-white px-8 py-3 text-[15px] font-semibold text-black hover:bg-white/90 transition-colors"
          style={{
            boxShadow: "0 14px 40px rgba(255,255,255,0.15)",
          }}
        >
          Try again
        </button>
        <Link
          href="/"
          className="rounded-full border border-white/20 px-8 py-3 text-[15px] font-medium text-white hover:bg-white/5 transition-colors"
        >
          Back to home
        </Link>
      </div>

      {error.digest && (
        <p className="mt-16 text-[10px] text-white/20 font-mono">
          Reference: {error.digest}
        </p>
      )}
    </main>
  );
}
