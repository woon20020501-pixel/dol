"use client";

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { GLOSSARY, type GlossaryTerm } from "@/lib/glossary";

/**
 * Glossary — inline "what is this?" tooltip.
 *
 * Renders a tiny question-mark glyph right next to a jargon term in
 * running text. Hovering (desktop) or tapping (mobile / keyboard)
 * reveals a small card containing the plain-English definition from
 * `src/lib/glossary.ts`. The card auto-places above or below the
 * trigger based on available viewport space, so it never gets
 * clipped by the bottom of the screen.
 *
 * Usage:
 *   <>You keep your money in a <Glossary term="wallet" inline>wallet</Glossary>.</>
 *
 * By default the component renders only the "?" icon. Pass children
 * with `inline` to wrap an arbitrary word so the entire word becomes
 * the hover target (the icon still sits at the end).
 *
 * Anti-jank:
 *   - No layout shift: tooltip is absolutely positioned on an
 *     inline-flex wrapper with `align-middle`
 *   - Hover open + Escape close + outside-click close all use
 *     plain React event handlers; no RAF loops
 *   - Placement recomputed only on open, not on every pointermove
 *   - `aria-describedby` pointer linking screen readers to the
 *     tooltip content when it's open, cleared otherwise
 *
 * Safety: the definition bodies in glossary.ts are plain strings
 * rendered as text nodes, never as HTML. No injection surface.
 */

interface GlossaryProps {
  term: GlossaryTerm;
  /** Wrap a word: the whole word becomes the hover target + gets a
      subtle dotted underline. When omitted, only the "?" icon is
      rendered, which is preferred for headlines and short copy. */
  children?: React.ReactNode;
}

export function Glossary({ term, children }: GlossaryProps) {
  const entry = GLOSSARY[term];
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<"above" | "below">("below");
  const wrapperRef = useRef<HTMLSpanElement>(null);
  const tooltipId = useId();

  // Recompute placement every time the tooltip opens. We measure the
  // wrapper's bounding rect and compare the space above and below to
  // the card's expected height (~140px including padding), falling
  // back to "below" when both sides have enough room.
  const recomputePlacement = useCallback(() => {
    const el = wrapperRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vh = window.innerHeight;
    const spaceAbove = rect.top;
    const spaceBelow = vh - rect.bottom;
    const needed = 170; // tooltip card max height estimate
    if (spaceBelow >= needed) setPlacement("below");
    else if (spaceAbove >= needed) setPlacement("above");
    else setPlacement(spaceAbove > spaceBelow ? "above" : "below");
  }, []);

  useEffect(() => {
    if (!open) return;
    recomputePlacement();
    // Also recompute on resize / scroll so a viewport change while
    // the tooltip is open doesn't clip it.
    const handler = () => recomputePlacement();
    window.addEventListener("resize", handler);
    window.addEventListener("scroll", handler, { passive: true });
    return () => {
      window.removeEventListener("resize", handler);
      window.removeEventListener("scroll", handler);
    };
  }, [open, recomputePlacement]);

  // Escape to close
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  // Outside click / tap to close — useful on touch devices where
  // `mouseleave` never fires.
  useEffect(() => {
    if (!open) return;
    const onOutside = (e: MouseEvent) => {
      if (
        wrapperRef.current &&
        !wrapperRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onOutside);
    return () => window.removeEventListener("mousedown", onOutside);
  }, [open]);

  if (!entry) return children ?? null;

  return (
    // Plain `relative` span — NOT inline-flex. We measured earlier
    // that inline-flex on the wrapper pushes the glyph ~7 px off the
    // surrounding paragraph's text baseline because the flex
    // container's baseline comes from its first child (the tiny
    // button with `leading-none 9 px`), not from the parent text.
    // Keeping the wrapper as a regular inline span lets every child
    // flow on the natural text baseline, and we put `align-middle`
    // on the icon button so it sits optically centered on the line.
    <span
      ref={wrapperRef}
      className="relative"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      {children && (
        <span className="underline decoration-white/20 decoration-dotted underline-offset-4 cursor-help">
          {children}
        </span>
      )}
      <button
        type="button"
        onClick={(e) => {
          e.preventDefault();
          setOpen((v) => !v);
        }}
        aria-label={`What is ${entry.label}?`}
        aria-expanded={open}
        aria-describedby={open ? tooltipId : undefined}
        className="ml-1 inline-flex h-[14px] w-[14px] items-center justify-center rounded-full border border-white/20 align-middle text-[9px] font-bold leading-none text-white/50 transition-colors hover:border-white/50 hover:text-white"
      >
        ?
      </button>
      {open && (
        <span
          id={tooltipId}
          role="tooltip"
          className={`pointer-events-none absolute left-1/2 z-50 w-64 max-w-[calc(100vw-2rem)] -translate-x-1/2 rounded-xl border border-white/10 bg-[#0a0a0a]/95 p-4 text-left backdrop-blur-md ${
            placement === "above" ? "bottom-full mb-2" : "top-full mt-2"
          }`}
          style={{
            boxShadow: "0 24px 60px rgba(0, 0, 0, 0.55)",
          }}
        >
          <span className="block text-[10px] font-semibold uppercase tracking-[0.14em] text-white/40">
            {entry.label}
          </span>
          <span className="mt-1.5 block text-[13px] font-semibold leading-snug text-white">
            {entry.short}
          </span>
          <span className="mt-1.5 block text-[11.5px] leading-relaxed text-white/55">
            {entry.long}
          </span>
        </span>
      )}
    </span>
  );
}
