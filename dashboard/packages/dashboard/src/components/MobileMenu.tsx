"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { motion, AnimatePresence } from "framer-motion";
import { Menu, X } from "lucide-react";

/**
 * MobileMenu — hamburger drawer for phones.
 *
 * The landing + /my-dol headers hide secondary nav links below the
 * `sm` (640 px) breakpoint to avoid the nav row overflowing on phones.
 * Without a mobile-only menu, phone users can only see the logo,
 * Login, and WalletChip — Docs/FAQ/Operator were unreachable unless
 * you typed the URL by hand. This component plugs that hole.
 *
 * Behavior:
 *   - Renders a Menu button only below `sm`
 *   - Opens a full-width sliding panel anchored to the top below the
 *     header bar
 *   - Closes on: link click, Escape, backdrop tap, route change
 *   - Body scroll is locked while open (prevents background jitter)
 *   - Focus is trapped to the panel while it's open (a11y)
 *
 * Kept dependency-light: no Radix / HeadlessUI. framer-motion for the
 * slide animation, lucide icons for the hamburger + close glyphs.
 */

export type MobileMenuLink = {
  href: string;
  label: string;
  external?: boolean;
};

export function MobileMenu({ links }: { links: MobileMenuLink[] }) {
  const [open, setOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  // Lock body scroll while open
  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  // Autofocus panel on open so keyboard users land inside it
  useEffect(() => {
    if (open && panelRef.current) {
      const first = panelRef.current.querySelector<HTMLElement>(
        "a, button, [tabindex]",
      );
      first?.focus();
    }
  }, [open]);

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex h-11 w-11 items-center justify-center rounded-full text-white/70 transition-colors hover:bg-white/10 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/40 sm:hidden"
        aria-label="Open menu"
        aria-expanded={open}
        aria-controls="mobile-menu-panel"
      >
        <Menu className="h-5 w-5" />
      </button>

      <AnimatePresence>
        {open && (
          <>
            {/* Backdrop */}
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.2 }}
              onClick={() => setOpen(false)}
              className="fixed inset-0 z-[60] bg-black/70 backdrop-blur-sm sm:hidden"
              aria-hidden="true"
            />

            {/* Panel — slides down from top, anchored below header */}
            <motion.div
              id="mobile-menu-panel"
              ref={panelRef}
              role="dialog"
              aria-modal="true"
              aria-label="Site navigation"
              initial={{ y: -16, opacity: 0 }}
              animate={{ y: 0, opacity: 1 }}
              exit={{ y: -16, opacity: 0 }}
              transition={{ duration: 0.22, ease: [0.2, 0.9, 0.25, 1] }}
              className="fixed left-3 right-3 top-[68px] z-[61] rounded-2xl border border-white/10 bg-[#0b0b0f]/95 p-4 backdrop-blur-xl sm:hidden"
            >
              <div className="flex items-center justify-between pb-2">
                <span className="text-[11px] uppercase tracking-[0.16em] text-white/40">
                  Menu
                </span>
                <button
                  type="button"
                  onClick={() => setOpen(false)}
                  className="inline-flex h-11 w-11 items-center justify-center rounded-full text-white/60 transition-colors hover:bg-white/10 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/40"
                  aria-label="Close menu"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              <nav className="mt-2 flex flex-col" aria-label="Mobile primary">
                {links.map((link, i) => (
                  <Link
                    key={link.href}
                    href={link.href}
                    {...(link.external
                      ? { target: "_blank", rel: "noopener noreferrer" }
                      : {})}
                    onClick={() => setOpen(false)}
                    className="flex items-center justify-between rounded-xl px-3 py-3 text-[15px] font-medium text-white/90 transition-colors hover:bg-white/[0.06]"
                    style={{
                      animationDelay: `${60 + i * 30}ms`,
                    }}
                  >
                    <span>{link.label}</span>
                    <span aria-hidden="true" className="text-white/30">
                      &rarr;
                    </span>
                  </Link>
                ))}
              </nav>
            </motion.div>
          </>
        )}
      </AnimatePresence>
    </>
  );
}
