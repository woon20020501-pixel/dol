"use client";

import { useEffect, useRef } from "react";

interface LiveCounterProps {
  initial: number;
  apy: number; // 0.075 = 7.5%
  decimals?: number;
  className?: string;
}

const SECONDS_PER_YEAR = 365 * 24 * 3600;

/**
 * LiveCounter — continuously compounding balance display.
 *
 * Zero React reconciliation in the per-frame path:
 *   useRef + requestAnimationFrame + element.innerText mutation.
 *
 * Verify with React DevTools Profiler: this component must show
 * 0 renders/sec during RAF loop. Any useState in the hot path would
 * trigger reconciliation at 60Hz and break the render budget.
 */
export default function LiveCounter({
  initial,
  apy,
  decimals = 4,
  className = "",
}: LiveCounterProps) {
  const ref = useRef<HTMLSpanElement>(null);
  const startRef = useRef({ t: performance.now(), principal: initial });

  useEffect(() => {
    startRef.current = { t: performance.now(), principal: initial };
    let raf = 0;
    const loop = () => {
      const elapsedSec = (performance.now() - startRef.current.t) / 1000;
      const yearsElapsed = elapsedSec / SECONDS_PER_YEAR;
      const v = startRef.current.principal * Math.exp(apy * yearsElapsed);
      if (ref.current) ref.current.innerText = v.toFixed(decimals);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [initial, apy, decimals]);

  return (
    <span
      ref={ref}
      className={className}
      style={{ fontVariantNumeric: "tabular-nums" }}
    >
      {initial.toFixed(decimals)}
    </span>
  );
}
