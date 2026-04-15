"use client";

import { useEffect, useState } from "react";

/**
 * SectionNav — right-rail vertical dot navigation.
 *
 * One tiny dot per top-level landing section. The currently-visible
 * section's dot scales up and highlights; hovering shows its label
 * to the left with no layout shift. Clicking scrolls to the section
 * via a hash anchor, which Next.js handles smoothly out of the box.
 *
 * Tracking:
 *   - IntersectionObserver with a tall `rootMargin` (-40% / -40%)
 *     so the "active" dot flips when the section's midpoint crosses
 *     the viewport midpoint — matches the intuition of "you are
 *     currently reading this section"
 *   - `once: false` — we stay subscribed and update on every scroll
 *
 * Visibility:
 *   - Hidden below md (<768 px) so phones don't eat screen real
 *     estate with a navigation rail
 *   - `pointer-events-none` on the outer wrapper lets the nav sit
 *     over the page without stealing clicks outside the actual dots
 *
 * Apple / Linear keynote pattern, scaled down to Dol's palette.
 */

const SECTIONS = [
  { id: "hero", label: "Home" },
  { id: "why", label: "Why" },
  { id: "grow", label: "Live" },
  { id: "simulate", label: "Try it" },
  { id: "how", label: "How" },
  { id: "trust", label: "Trust" },
  { id: "health", label: "Status" },
  { id: "cta", label: "Join" },
];

export function SectionNav() {
  const [active, setActive] = useState<string>(SECTIONS[0].id);

  useEffect(() => {
    // Observe each anchored section. We key off the id attributes
    // that page.tsx adds to every landing <section>. If an id is
    // missing we silently skip it — no console spam.
    const observer = new IntersectionObserver(
      (entries) => {
        // Among entries that just became intersecting, pick the one
        // closest to the viewport midpoint.
        const visible = entries.filter((e) => e.isIntersecting);
        if (visible.length === 0) return;
        const best = visible.reduce((a, b) =>
          a.intersectionRatio > b.intersectionRatio ? a : b,
        );
        setActive(best.target.id);
      },
      {
        // Midpoint-ish detection: only count a section as "active"
        // when its center has scrolled into the viewport's middle.
        rootMargin: "-40% 0px -40% 0px",
        threshold: [0, 0.25, 0.5, 0.75, 1],
      },
    );

    for (const s of SECTIONS) {
      const el = document.getElementById(s.id);
      if (el) observer.observe(el);
    }

    return () => observer.disconnect();
  }, []);

  return (
    <nav
      aria-label="Section navigation"
      className="pointer-events-none fixed right-6 top-1/2 z-30 hidden -translate-y-1/2 flex-col gap-5 md:flex"
    >
      {SECTIONS.map((s) => {
        const isActive = s.id === active;
        return (
          <a
            key={s.id}
            href={`#${s.id}`}
            aria-label={`Jump to ${s.label}`}
            className="pointer-events-auto group flex items-center justify-end gap-3"
          >
            <span
              className={`text-[10px] font-medium uppercase tracking-[0.18em] transition-all duration-300 ${
                isActive
                  ? "text-white opacity-100"
                  : "text-white/50 opacity-0 group-hover:opacity-100"
              }`}
              aria-hidden
            >
              {s.label}
            </span>
            <span
              className={`rounded-full transition-all duration-300 ${
                isActive
                  ? "h-2 w-2 bg-white"
                  : "h-1.5 w-1.5 bg-white/30 group-hover:bg-white/70"
              }`}
              aria-hidden
            />
          </a>
        );
      })}
    </nav>
  );
}
