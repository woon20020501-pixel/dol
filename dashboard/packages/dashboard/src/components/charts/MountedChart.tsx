"use client";

import { type ReactElement } from "react";
import { ResponsiveContainer } from "recharts";

/**
 * Thin wrapper over Recharts' `<ResponsiveContainer>` that passes an
 * explicit pixel `height` instead of `"100%"`, which silences
 * recharts' "width(-1) height(-1)" first-render warning.
 *
 * Recharts 3.8.1 source (es6/component/responsiveContainerUtils.js):
 *
 *     var calculatedHeight = isPercent(height)
 *       ? containerHeight        // from ResizeObserver, -1 until measured
 *       : Number(height);        // immediate, no measurement
 *
 *     warn(calculatedWidth > 0 || calculatedHeight > 0, "...");
 *
 * So passing `height={280}` (a number) makes `calculatedHeight = 280`
 * on first render, the OR condition is true, and the warning never
 * fires. Width stays at `"100%"` (flexes to parent) — that's fine
 * because the warning only needs ONE dimension > 0.
 *
 * Callers must wrap this in a parent div that reserves the same
 * vertical space (e.g. `h-[280px]`) so CLS stays at zero.
 */
export function MountedChart({
  height,
  children,
}: {
  height: number;
  children: ReactElement;
}) {
  return (
    <ResponsiveContainer width="100%" height={height}>
      {children}
    </ResponsiveContainer>
  );
}
