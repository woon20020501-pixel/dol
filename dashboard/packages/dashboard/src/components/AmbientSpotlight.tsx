"use client";

import { useEffect, useRef } from "react";

/**
 * AmbientSpotlight — cursor-follow specular highlight.
 *
 * Sits as an absolutely-positioned overlay inside its parent. A
 * radial gradient is drawn centered on CSS custom properties
 * `--mx` / `--my`, and we write those props via direct DOM mutation
 * on every frame while the cursor is inside the wrapper. Compositor
 * draws the gradient; nothing reflows, nothing repaints outside the
 * overlay layer.
 *
 * Anti-jank:
 *   - RAF-throttled: the pointermove event can fire at up to
 *     2000 Hz on high-refresh trackpads; we coalesce to one style
 *     mutation per animation frame
 *   - Zero React state — `element.style.setProperty` directly,
 *     so there's no reconciliation in the hot path
 *   - `prefers-reduced-motion` short-circuits the effect entirely
 *     and the spotlight stays parked at dead center (still visible
 *     but not chasing the cursor)
 *   - `pointer-events-none` on the overlay so the spotlight never
 *     intercepts clicks
 *
 * Usage:
 *   <div className="relative ...">
 *     <AmbientSpotlight />
 *     ...content...
 *   </div>
 *
 * The component positions itself absolutely to fill the nearest
 * positioned ancestor.
 */

interface AmbientSpotlightProps {
  /** Diameter of the highlight in pixels. Default 520. */
  size?: number;
  /** Peak opacity at the center. Default 0.14 (subtle). */
  intensity?: number;
  /** Gradient tint — hex or rgb string. Default white. */
  color?: string;
}

export function AmbientSpotlight({
  size = 520,
  intensity = 0.14,
  color = "rgba(255, 255, 255, 1)",
}: AmbientSpotlightProps) {
  const ref = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  const pendingRef = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // The spotlight's parent is the nearest positioned ancestor.
    // We listen for pointermove ON THAT PARENT so the tracking
    // area matches the visual bounds perfectly.
    const parent = el.parentElement;
    if (!parent) return;

    if (
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      // Reduced-motion: park the spotlight in the middle and bail.
      el.style.setProperty("--mx", "50%");
      el.style.setProperty("--my", "50%");
      el.style.opacity = "1";
      return;
    }

    const onMove = (e: PointerEvent) => {
      pendingRef.current = { x: e.clientX, y: e.clientY };
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const pending = pendingRef.current;
        if (!pending) return;
        const rect = parent.getBoundingClientRect();
        const x = ((pending.x - rect.left) / rect.width) * 100;
        const y = ((pending.y - rect.top) / rect.height) * 100;
        el.style.setProperty("--mx", `${x.toFixed(2)}%`);
        el.style.setProperty("--my", `${y.toFixed(2)}%`);
        el.style.opacity = "1";
      });
    };

    const onLeave = () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      // Fade the spotlight out when the cursor leaves. CSS transition
      // handles the decay so we don't script a per-frame loop.
      el.style.opacity = "0";
    };

    parent.addEventListener("pointermove", onMove);
    parent.addEventListener("pointerleave", onLeave);
    return () => {
      parent.removeEventListener("pointermove", onMove);
      parent.removeEventListener("pointerleave", onLeave);
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  return (
    <div
      ref={ref}
      aria-hidden
      className="pointer-events-none absolute inset-0 -z-10"
      style={{
        // Initial rest position — dead center, invisible until the
        // cursor enters. `--mx`/`--my` are mutated by the effect
        // above without triggering React reconciliation.
        ["--mx" as string]: "50%",
        ["--my" as string]: "50%",
        opacity: 0,
        transition: "opacity 450ms ease-out",
        background: `radial-gradient(
          ${size}px circle at var(--mx) var(--my),
          ${color.replace(/[\d.]+\)$/, `${intensity})`)} 0%,
          ${color.replace(/[\d.]+\)$/, `${intensity * 0.45})`)} 22%,
          transparent 60%
        )`,
      }}
    />
  );
}
