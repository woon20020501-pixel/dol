"use client";

import { useEffect, useRef } from "react";

/**
 * MagneticCard — 3D cursor-follow tilt wrapper.
 *
 * Wraps its children in an absolutely-positioned div whose CSS
 * `transform` is mutated directly via requestAnimationFrame as the
 * cursor moves over the element's bounding rect. Maximum tilt at the
 * corners, zero tilt at the center. The transform is
 * `perspective(N) rotateX rotateY translateZ(0)`, all compositor-only,
 * so there's no layout or paint cost per frame.
 *
 * Anti-jank rules:
 *   - RAF-throttled: at most one transform write per animation frame
 *     even if mousemove fires 144 times per second
 *   - No React state: direct DOM style mutation, zero reconciliation
 *     in the hot path (same budgetary pattern as LiveCounter)
 *   - Spring return: on mouseleave, the card decays to rest over
 *     ~300 ms via CSS transition, not a per-frame script
 *   - prefers-reduced-motion: we never attach the handler at all
 *     when the media query is true, so keyboard/touch users and
 *     anyone with motion sickness get a flat, stable card
 *   - Touch skip: mousemove only — `touchstart` is ignored so mobile
 *     doesn't get locked into a tilted state after a tap
 *
 * Inspired by the Stripe / Linear / Vercel homepage card hovers,
 * but hand-rolled so we don't pull in a 12 kB tilt library for one
 * effect.
 */

interface MagneticCardProps {
  children: React.ReactNode;
  /** Maximum tilt in degrees at the card's corners. Default 4°. */
  maxTilt?: number;
  /** Perspective distance in px. Higher = flatter tilt. Default 1000. */
  perspective?: number;
  className?: string;
  /** Extra inline style — merged with the internal transform rest state. */
  style?: React.CSSProperties;
}

export function MagneticCard({
  children,
  maxTilt = 4,
  perspective = 1000,
  className = "",
  style,
}: MagneticCardProps) {
  const ref = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  const pendingRef = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // Respect user motion preferences — bail before binding.
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      return;
    }

    const reset = () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      pendingRef.current = null;
      el.style.transform = `perspective(${perspective}px) rotateX(0deg) rotateY(0deg) translateZ(0)`;
    };

    const onMove = (e: MouseEvent) => {
      pendingRef.current = { x: e.clientX, y: e.clientY };
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const pending = pendingRef.current;
        if (!pending) return;
        const rect = el.getBoundingClientRect();
        // Normalized offset from card center, in the range [-1, 1]
        const nx = (pending.x - rect.left) / rect.width - 0.5;
        const ny = (pending.y - rect.top) / rect.height - 0.5;
        const tiltY = nx * maxTilt * 2;
        const tiltX = -ny * maxTilt * 2;
        el.style.transform = `perspective(${perspective}px) rotateX(${tiltX.toFixed(
          2,
        )}deg) rotateY(${tiltY.toFixed(
          2,
        )}deg) translateZ(0)`;
      });
    };

    const onLeave = () => reset();

    el.addEventListener("mousemove", onMove);
    el.addEventListener("mouseleave", onLeave);
    return () => {
      el.removeEventListener("mousemove", onMove);
      el.removeEventListener("mouseleave", onLeave);
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [maxTilt, perspective]);

  return (
    <div
      ref={ref}
      className={className}
      style={{
        // Rest transform — `translateZ(0)` promotes to its own layer
        // so the browser doesn't repaint the subtree on every tilt
        // update. `transform-style: preserve-3d` lets nested elements
        // sit at their own depth if we ever add parallax inside.
        transform: `perspective(${perspective}px) rotateX(0deg) rotateY(0deg) translateZ(0)`,
        transformStyle: "preserve-3d",
        transition: "transform 360ms cubic-bezier(0.05, 0.7, 0.1, 1)",
        willChange: "transform",
        ...style,
      }}
    >
      {children}
    </div>
  );
}
