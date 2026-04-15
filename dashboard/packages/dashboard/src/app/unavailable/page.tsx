"use client";

import { Suspense, useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import DolHeroImage from "@/components/DolHeroImage";

/**
 * Geo-block landing. Hit via edge middleware rewrite when the
 * visitor's country isn't in the Tier A whitelist (VN/TR/PH/MX/AR).
 *
 * The "from" query param is set by the middleware so we can show the
 * detected country — purely informational, the user can't override it.
 *
 * Waitlist is deliberately client-side only. We don't want to wire up
 * a backend endpoint until legal signs off on what we're allowed to
 * do with the addresses. For now, `mailto:` routes to the team.
 */
export default function UnavailablePage() {
  return (
    <Suspense
      fallback={
        <main className="min-h-screen bg-black text-white flex items-center justify-center" />
      }
    >
      <UnavailableInner />
    </Suspense>
  );
}

function UnavailableInner() {
  const params = useSearchParams();
  const from = params.get("from") || "";
  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState(false);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!email || !/.+@.+\..+/.test(email)) return;
    const subject = encodeURIComponent("Dol waitlist");
    const body = encodeURIComponent(
      `Country: ${from || "unknown"}\nEmail: ${email}\n`,
    );
    window.location.href = `mailto:waitlist@dol.money?subject=${subject}&body=${body}`;
    setSubmitted(true);
  };

  return (
    <main className="min-h-screen bg-black text-white flex flex-col items-center justify-center px-6 py-12">
      <DolHeroImage size={180} />

      <h1
        className="mt-12 text-4xl md:text-5xl font-bold text-white text-center leading-[1.05]"
        style={{ letterSpacing: "-0.04em" }}
      >
        Not yet available here.
      </h1>

      <p className="mt-4 max-w-md text-center text-[15px] text-white/50">
        Dol is currently live in a handful of countries while we finish
        our compliance work. We&apos;re expanding as fast as we can.
      </p>

      {!submitted ? (
        <form
          onSubmit={onSubmit}
          className="mt-10 flex w-full max-w-sm flex-col gap-3"
        >
          <label htmlFor="waitlist-email" className="sr-only">
            Email address
          </label>
          <input
            id="waitlist-email"
            type="email"
            required
            autoComplete="email"
            placeholder="you@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="w-full rounded-full border border-white/15 bg-white/5 px-5 py-3 text-[15px] text-white placeholder-white/30 focus:border-white/40 focus:outline-none"
          />
          <button
            type="submit"
            className="w-full rounded-full bg-white px-8 py-3 text-[15px] font-semibold text-black transition-colors hover:bg-white/90"
            style={{ boxShadow: "0 14px 40px rgba(255,255,255,0.15)" }}
          >
            Join the waitlist
          </button>
          <p className="text-center text-xs text-white/30">
            We&apos;ll email you once Dol is available in your region.
          </p>
        </form>
      ) : (
        <div className="mt-10 max-w-sm text-center text-[15px] text-white/70">
          Thanks — we&apos;ll be in touch.
        </div>
      )}

      <Link
        href="/legal/terms"
        className="mt-8 text-sm text-white/40 underline-offset-4 hover:text-white/70 hover:underline"
      >
        Read our terms
      </Link>
    </main>
  );
}
