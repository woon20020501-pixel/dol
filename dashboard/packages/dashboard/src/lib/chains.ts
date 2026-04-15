import { baseSepolia } from "wagmi/chains";

/**
 * Single source of truth for the chain the app targets.
 *
 * Before: `Number(process.env.NEXT_PUBLIC_CHAIN_ID || "84532")` was
 * sprinkled in useDeposit, while useDolWithdraw / LiveVaultTicker /
 * SystemHealthSection imported `baseSepolia.id` directly. If the env
 * var is ever malformed or a different number than baseSepolia.id,
 * different hooks would disagree about what "the current chain" is,
 * which silently breaks write paths with cryptic wrong-network errors.
 *
 * This file centralizes it. Every place that needs a chain id should
 * import `TARGET_CHAIN_ID` (or `TARGET_CHAIN`) from here, not read
 * env / import from `wagmi/chains` directly.
 *
 * Promotion to another chain later = edit this file + `wagmi.ts`,
 * one change, grep-able.
 */

export const TARGET_CHAIN = baseSepolia;
export const TARGET_CHAIN_ID = baseSepolia.id;

/**
 * Optional env-override escape hatch for local forks / devnets. If
 * NEXT_PUBLIC_CHAIN_ID is set and parses to a valid number AND matches
 * TARGET_CHAIN.id we accept it; if it mismatches we keep the constant
 * and warn (dev only), which prevents silent desync between hooks.
 */
export function assertExpectedChainId(value: number | string | undefined): number {
  if (value === undefined || value === null || value === "") return TARGET_CHAIN_ID;
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed)) return TARGET_CHAIN_ID;
  if (parsed !== TARGET_CHAIN_ID && process.env.NODE_ENV !== "production") {
    // eslint-disable-next-line no-console
    console.warn(
      `[chains] NEXT_PUBLIC_CHAIN_ID=${parsed} disagrees with TARGET_CHAIN_ID=${TARGET_CHAIN_ID}. ` +
        "Ignoring env value to avoid hook desync.",
    );
  }
  return TARGET_CHAIN_ID;
}
