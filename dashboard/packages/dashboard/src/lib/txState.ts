/**
 * Pure helpers for aggregating wagmi tx state across multiple
 * writeContract + waitForTransactionReceipt pairs.
 *
 * Extracted from useDeposit / useDolWithdraw / useWithdraw so the
 * aggregation logic can be exercised under vitest without spinning up
 * wagmi providers.
 *
 * Contract: these helpers are STATELESS — every call returns the same
 * output for the same inputs. Never touch window / localStorage /
 * wagmi hooks from here.
 */

export type TxFlow = {
  /** True when useWaitForTransactionReceipt throws for this flow — in
   *  wagmi core 2.22.1 this means the tx was mined but reverted. */
  isReverted: boolean;
  /** The submitted tx hash (from useWriteContract's `data`). */
  hash: `0x${string}` | undefined;
};

/**
 * Picks the first reverted flow's hash so the UI can deep-link to
 * basescan. Returns null if no flow is reverted. If multiple flows
 * somehow report reverted simultaneously (shouldn't happen in
 * practice — flows share disabled states in the UI), earlier flows
 * in the list win.
 */
export function pickRevertedHash(
  flows: readonly TxFlow[],
): `0x${string}` | null {
  for (const flow of flows) {
    if (flow.isReverted && flow.hash) return flow.hash;
  }
  return null;
}

/**
 * True if any flow is reverted. Separate from pickRevertedHash
 * because the consumer UI may render a "Reverted on-chain" badge
 * even before a hash is available (race between isReverted flipping
 * and hash being cleared).
 */
export function anyReverted(flows: readonly TxFlow[]): boolean {
  return flows.some((f) => f.isReverted);
}

/**
 * Merge a writeContract error (wallet rejection, gas estimate) with
 * a receipt-query error (on-chain revert). writeContract errors take
 * precedence because they fire FIRST in the tx lifecycle — if the
 * user rejects the wallet prompt, there is no tx hash and no receipt
 * ever gets queried.
 *
 * The two wagmi error union types are structurally distinct
 * (WriteContractErrorType vs WaitForTransactionReceiptErrorType), so
 * we accept Error | null | undefined and return the looser type.
 * Consumers funnel the result through translateError() which tolerates
 * both shapes.
 */
export function mergeTxError(
  writeErr: Error | null | undefined,
  receiptErr: Error | null | undefined,
): Error | null {
  if (writeErr) return writeErr;
  if (receiptErr) return receiptErr;
  return null;
}

/**
 * True iff the given error is an on-chain revert (as opposed to an
 * RPC failure, timeout, or network hiccup). Critical distinction:
 * wagmi's `useWaitForTransactionReceipt` sets `isError: true` for
 * BOTH cases, but the UI should only label something "Reverted
 * on-chain" when it actually was — otherwise we mislabel a flaky RPC
 * as a contract-level failure, which is user-confusing and leads to
 * wrong debugging intuition.
 *
 * Detection rule (derived from wagmi core 2.22.1's
 * actions/waitForTransactionReceipt.ts):
 *   - Revert path: `throw new Error(reason)` → plain Error whose
 *     `.name === "Error"` and whose `.message` is the decoded revert
 *     reason (or the literal string "unknown reason" when the on-chain
 *     callstatic couldn't return a selector).
 *   - RPC/transport errors: viem throws named subclasses like
 *     `HttpRequestError`, `WaitForTransactionReceiptTimeoutError`,
 *     `TransactionReceiptNotFoundError`, etc. — all have distinct
 *     `.name` values.
 *
 * We match on `.name === "Error"` plus a non-empty message. If wagmi
 * ever changes their error shape this heuristic will gracefully fall
 * back to "not a revert" — safer than falsely flagging RPC errors.
 */
export function isRevertError(err: unknown): boolean {
  if (!(err instanceof Error)) return false;
  if (err.name !== "Error") return false;
  return typeof err.message === "string" && err.message.length > 0;
}
