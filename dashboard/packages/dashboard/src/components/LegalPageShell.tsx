import Link from "next/link";
import { type ReactNode } from "react";

/**
 * Shared chrome for /legal/* pages — header back link, centered
 * column, and a footer note that the document is v0.1 / pre-launch.
 *
 * Pages stay thin: they just hand a parsed markdown ReactNode to
 * `children` and pick a title.
 */
export function LegalPageShell({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <main className="min-h-screen bg-black text-white">
      <header className="sticky top-0 z-10 border-b border-white/5 bg-black/80 backdrop-blur">
        <div className="mx-auto flex max-w-2xl items-center justify-between px-6 py-4">
          <Link
            href="/"
            className="text-sm text-white/60 hover:text-white"
            aria-label="Back to home"
          >
            ← Dol
          </Link>
          <span className="text-xs uppercase tracking-wider text-white/30">
            {title}
          </span>
        </div>
      </header>

      <article className="mx-auto max-w-2xl px-6 pb-24 pt-8">
        {children}

        <footer className="mt-16 border-t border-white/5 pt-6 text-xs text-white/30">
          This document is a pre-launch draft (v0.1). It will be replaced by
          a final version reviewed by counsel before public launch. Questions:
          legal@dol.money.
        </footer>
      </article>
    </main>
  );
}
