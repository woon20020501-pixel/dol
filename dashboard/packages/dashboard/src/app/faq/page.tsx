import fs from "node:fs";
import path from "node:path";
import Link from "next/link";
import { FaqTabs } from "@/components/FaqTabs";
import { SiteFooter } from "@/components/SiteFooter";
import { parseFaq } from "@/lib/faq";

export const metadata = {
  title: "FAQ · Dol",
  description:
    "Questions people ask about Dol — short answers, organized by topic.",
};

/**
 * Top-level marketing-style FAQ.
 *
 * Distinct from the /docs/faq reference page shipped in : this
 * one is a tabbed, scannable, visitor-facing view modeled on
 * liminal.money/faq. Content is sourced from src/content/faq.md
 * (VP-authored, ~30 questions across 7 categories). Parsing happens
 * at build time via `fs.readFileSync` + the tiny parser in lib/faq.ts,
 * so the runtime bundle only carries the structured JSON.
 */
export default function FaqPage() {
  const md = fs.readFileSync(
    path.join(process.cwd(), "src/content/faq.md"),
    "utf8",
  );
  const categories = parseFaq(md);

  return (
    <main className="min-h-screen bg-black text-white">
      <header className="sticky top-0 z-20 border-b border-white/5 bg-black/80 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
          <Link
            href="/"
            className="text-sm font-semibold text-white/80 hover:text-white"
          >
            Dol
          </Link>
          <span className="text-xs uppercase tracking-[0.14em] text-white/30">
            FAQ
          </span>
        </div>
      </header>

      <article className="mx-auto max-w-3xl px-6 pb-24 pt-12 md:px-8">
        <h1
          className="text-4xl font-bold text-white"
          style={{ letterSpacing: "-0.03em" }}
        >
          FAQ
        </h1>
        <p className="mt-4 text-[15px] leading-relaxed text-white/70">
          The questions people ask about Dol — short answers, organized by
          topic. For deeper explanations, see the{" "}
          <Link
            href="/docs"
            className="text-white underline decoration-white/30 underline-offset-4 hover:decoration-white"
          >
            docs
          </Link>
          .
        </p>

        <FaqTabs categories={categories} />

        <div className="mt-20 border-t border-white/5 pt-8 text-[14px] text-white/60">
          Did not find what you were looking for?{" "}
          <Link
            href="/docs/more/support"
            className="text-white underline decoration-white/30 underline-offset-4 hover:decoration-white"
          >
            Contact us
          </Link>{" "}
          or read the full{" "}
          <Link
            href="/docs"
            className="text-white underline decoration-white/30 underline-offset-4 hover:decoration-white"
          >
            documentation
          </Link>
          .
        </div>
      </article>

      <SiteFooter />
    </main>
  );
}
