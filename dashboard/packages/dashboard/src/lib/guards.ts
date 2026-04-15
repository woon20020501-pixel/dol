/**
 * Runtime type + value guards for untrusted inputs (localStorage,
 * URL params, config.json, etc.). Every boundary that crosses from
 * "string from disk" into "typed object we trust" should go through
 * one of these. Failing the check means drop the item and log — do
 * not silently accept malformed data.
 *
 * Motivation: XSS or supply-chain compromise could plant crafted
 * payloads in localStorage or contracts.json. Runtime validation
 * makes the attack surface ride explicit types instead of implicit
 * trust in `JSON.parse` + `as` casts.
 */

import type { Address } from "viem";

/** 0x + 40 hex chars, case-insensitive */
const ADDRESS_RE = /^0x[0-9a-fA-F]{40}$/;
/** 0x + 64 hex chars */
const TX_HASH_RE = /^0x[0-9a-fA-F]{64}$/;
/** Digits only (uint256 as decimal string) */
const UINT_DECIMAL_RE = /^\d+$/;

export function isValidAddress(value: unknown): value is Address {
  return typeof value === "string" && ADDRESS_RE.test(value);
}

export function assertAddress(
  value: unknown,
  label: string,
): asserts value is Address {
  if (!isValidAddress(value)) {
    throw new Error(
      `[guard] ${label} is not a valid address: ${String(value)}`,
    );
  }
}

export function isValidTxHash(
  value: unknown,
): value is `0x${string}` {
  return typeof value === "string" && TX_HASH_RE.test(value);
}

export function isValidRedeemId(value: unknown): value is string {
  // Stored as decimal string in localStorage
  return typeof value === "string" && UINT_DECIMAL_RE.test(value);
}

export function isFiniteNonNegNumber(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= 0
  );
}

/** Logged tx schema — match shape used by useTxHistory */
interface LoggedTxShape {
  hash: `0x${string}`;
  type: string;
  amount: number;
  timestamp: number;
  status: string;
}

const VALID_TX_TYPES = new Set([
  "deposit",
  "redeem-scheduled",
  "redeem-instant",
  "claim",
  "approve",
]);

const VALID_TX_STATUSES = new Set(["pending", "confirmed", "failed"]);

export function isValidLoggedTx(v: unknown): v is LoggedTxShape {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    isValidTxHash(o.hash) &&
    typeof o.type === "string" &&
    VALID_TX_TYPES.has(o.type) &&
    isFiniteNonNegNumber(o.amount) &&
    isFiniteNonNegNumber(o.timestamp) &&
    typeof o.status === "string" &&
    VALID_TX_STATUSES.has(o.status)
  );
}

/** Pending redeem schema — match shape used by useDolWithdraw */
interface PendingRedeemShape {
  requestId: string;
  shares: number;
  requestedAt: number;
}

export function isValidPendingRedeem(v: unknown): v is PendingRedeemShape {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    isValidRedeemId(o.requestId) &&
    isFiniteNonNegNumber(o.shares) &&
    isFiniteNonNegNumber(o.requestedAt)
  );
}

/**
 * Parse JSON from localStorage with validation. Returns filtered array
 * dropping any items that fail the per-item validator. Never throws —
 * bad data just becomes empty list.
 */
export function parseLocalStorageArray<T>(
  raw: string | null,
  validator: (x: unknown) => x is T,
): T[] {
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  return parsed.filter(validator);
}
