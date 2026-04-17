import Link from "next/link";

/**
 * Minimal site footer carrying legal links + copyright. Sits on every
 * top-level page (`/`, `/deposit`, `/my-dol`) so the legal pages are
 * always one click away — required by .
 *
 * Kept visually quiet so it doesn't compete with hero content. Mobile
 * stacks the links above the copyright line.
 */
export function SiteFooter() {
  return (
    <footer className="border-t border-white/[0.08] bg-black/60 py-8 text-xs text-white/55">
      <div className="mx-auto flex max-w-5xl flex-col items-center gap-4 px-6 sm:flex-row sm:justify-between">
        <nav
          className="flex flex-wrap items-center justify-center gap-x-6 gap-y-2"
          aria-label="Legal"
        >
          <Link
            href="/docs"
            className="transition-colors hover:text-white/90"
          >
            Docs
          </Link>
          <Link
            href="/faq"
            className="transition-colors hover:text-white/90"
          >
            FAQ
          </Link>
          <Link
            href="/legal/terms"
            className="transition-colors hover:text-white/90"
          >
            Terms
          </Link>
          <Link
            href="/legal/privacy"
            className="transition-colors hover:text-white/90"
          >
            Privacy
          </Link>
          <Link
            href="/legal/risk"
            className="transition-colors hover:text-white/90"
          >
            Risk
          </Link>
        </nav>

        <div className="text-center sm:text-right sm:max-w-sm">
          Dol is not a bank. Not FDIC insured. Not investment, legal, or
          tax advice. Crypto involves risk of loss. Do your own research.
        </div>
      </div>
    </footer>
  );
}
