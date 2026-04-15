"use client";

/**
 * Lazy loaders for heavy client-only components.
 *
 * These components are either:
 *   - only needed AFTER initial paint (modal shown on first visit)
 *   - only rendered in response to user interaction (cashout sheet
 *     on /my-dol)
 *
 * Wrapping them in `next/dynamic` with `ssr: false` pulls them out of
 * the initial JS bundle and into their own chunks that download when
 * they're actually mounted. The visible effect is a smaller
 * first-load JS across every route that imports these.
 *
 * NOT included here:
 *   - FirstDepositClickwrap — its public surface is a hook
 *     (`useTosAcceptance`), not a component, so it has to be imported
 *     synchronously. The modal element it returns is already behind
 *     an `open` flag, so its render cost is already gated.
 *
 * Re-exports are deliberate — consumers import from this module
 * instead of the real component files, so the deferral is transparent.
 */
import dynamic from "next/dynamic";

export const VisitGateModal = dynamic(
  () => import("./VisitGateModal").then((m) => ({ default: m.VisitGateModal })),
  { ssr: false },
);

export const CashoutSheet = dynamic(() => import("./CashoutSheet"), {
  ssr: false,
});

export const CommandPalette = dynamic(
  () => import("./CommandPalette").then((m) => ({ default: m.CommandPalette })),
  { ssr: false },
);
