"use client";

/**
 * Cross-component transaction event bus.
 *
 * Why: the dashboard has ~8 different places that read on-chain state
 * (useDolBalance, useVaultReads, LiveVaultTicker, SystemHealthSection,
 * NAV reporter card, pending withdraw list, etc.). When the user
 * completes a deposit / request-redeem / instant-redeem / claim tx
 * on any page, ALL of those reads have become stale — balance, vault
 * totalSupply, pricePerShare, and pending list can all flip.
 *
 * Before: each read hook relied on its own `refetchInterval` (10-15 s)
 * so the UI was out-of-sync for up to a full poll interval after any
 * tx. Users learned to hit Cmd+R to force a refresh.
 *
 * After: the write-side hooks emit `dol:tx-confirmed` on tx receipt.
 * Every read-side hook subscribes to this event plus the browser's
 * `visibilitychange` event (for back-from-another-tab refresh) and
 * calls its own `refetch()`. UI stays in sync within one RPC round
 * trip of any confirmation.
 *
 * Implementation: a window-level CustomEvent bus. No React context
 * required, no Provider hierarchy, works across route changes (since
 * `window` persists). SSR-safe via typeof-window guards.
 */

export type DolTxKind =
  | "deposit"
  | "approve"
  | "request-redeem"
  | "instant-redeem"
  | "claim-redeem";

export type DolTxEventDetail = {
  kind: DolTxKind;
  txHash?: `0x${string}`;
  at: number; // Date.now() when confirmed
};

const EVENT_NAME = "dol:tx-confirmed";

/**
 * Broadcast a tx-confirmation event. Call from the useEffect that
 * reacts to useWaitForTransactionReceipt's `isSuccess: true`.
 */
export function emitDolTxConfirmed(
  kind: DolTxKind,
  txHash?: `0x${string}`,
): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<DolTxEventDetail>(EVENT_NAME, {
      detail: { kind, txHash, at: Date.now() },
    }),
  );
}

/**
 * Subscribe to tx-confirmation events. Returns an unsubscribe fn so
 * React useEffect can clean up on unmount. Handler fires for every
 * tx kind; filter inside if you only care about specific ones.
 */
export function onDolTxConfirmed(
  handler: (detail: DolTxEventDetail) => void,
): () => void {
  if (typeof window === "undefined") return () => {};
  const listener = (e: Event) => {
    const ce = e as CustomEvent<DolTxEventDetail>;
    handler(ce.detail);
  };
  window.addEventListener(EVENT_NAME, listener);
  return () => window.removeEventListener(EVENT_NAME, listener);
}

/**
 * Convenience: subscribe to both tx confirmations AND the tab becoming
 * visible (user returning from another app / tab). Read hooks use this
 * so returning to the page forces fresh state even without a tx.
 */
export function onDolStateShouldRefresh(handler: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  const txUnsub = onDolTxConfirmed(() => handler());
  const visListener = () => {
    if (document.visibilityState === "visible") handler();
  };
  document.addEventListener("visibilitychange", visListener);
  return () => {
    txUnsub();
    document.removeEventListener("visibilitychange", visListener);
  };
}
