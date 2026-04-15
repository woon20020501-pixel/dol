/**
 * User-friendly error translator.
 *
 * Takes a wagmi/viem error (or any Error) and returns plain-English copy
 * suitable for a toast. NO blockchain jargon. NO revert strings. NO
 * "your money is safe" (see legal review — contradicts our at-risk
 * disclaimer).
 *
 * Returns a category so the UI can branch (e.g., show "Get test USDC"
 * link only for insufficient-balance errors).
 */

export type ErrorCategory =
  | "user_rejected"
  | "insufficient_usdc"
  | "insufficient_eth"
  | "wrong_network"
  | "network_glitch"
  | "contract_revert"
  | "unknown";

export interface UserError {
  category: ErrorCategory;
  title: string;
  description?: string;
}

/* Pattern match wagmi/viem error shapes. These libraries nest the raw
   reason inside several wrapper layers — we scan the whole toString(). */
export function translateError(err: unknown): UserError {
  const msg = errorToString(err).toLowerCase();

  // Case 1 — user rejected in wallet
  if (
    msg.includes("user rejected") ||
    msg.includes("user denied") ||
    msg.includes("rejected the request") ||
    msg.includes("userrejectedrequesterror")
  ) {
    return {
      category: "user_rejected",
      title: "No worries. Try again anytime.",
    };
  }

  // Case 2 — insufficient USDC (approve or transfer)
  if (
    msg.includes("transfer amount exceeds balance") ||
    msg.includes("insufficient balance") ||
    msg.includes("exceeds allowance")
  ) {
    return {
      category: "insufficient_usdc",
      title: "You need more USDC to buy this much Dol.",
    };
  }

  // Case 3 — insufficient ETH for gas
  if (
    msg.includes("insufficient funds for gas") ||
    msg.includes("insufficient funds") ||
    msg.includes("out of gas") ||
    msg.includes("intrinsic gas too low")
  ) {
    return {
      category: "insufficient_eth",
      title: "Out of gas. Get a little ETH first.",
    };
  }

  // Case 8 — wrong network
  if (
    msg.includes("chain mismatch") ||
    msg.includes("chain id mismatch") ||
    msg.includes("does not match the target chain") ||
    msg.includes("switch chain")
  ) {
    return {
      category: "wrong_network",
      title: "Dol lives on Base Sepolia. Switch?",
    };
  }

  // Case 7 — network glitch
  if (
    msg.includes("network request failed") ||
    msg.includes("failed to fetch") ||
    msg.includes("fetch failed") ||
    msg.includes("timeout") ||
    msg.includes("timed out") ||
    msg.includes("econnreset") ||
    msg.includes("rpc") ||
    msg.includes("http request failed")
  ) {
    return {
      category: "network_glitch",
      title: "Connection hiccup. Please try again.",
    };
  }

  // Case 6 — generic contract revert (execution reverted, custom error, etc.)
  if (
    msg.includes("execution reverted") ||
    msg.includes("contractfunctionexecutionerror") ||
    msg.includes("custom error") ||
    msg.includes("revert")
  ) {
    return {
      category: "contract_revert",
      // CRITICAL: do NOT use "your money is safe" — contradicts at-risk
      // disclaimer. Factual framing: tx reverts are atomic, so no Dol
      // was created and no USDC moved.
      title: "Something went wrong.",
      description: "No Dol was bought. Please try again.",
    };
  }

  // Fallback
  return {
    category: "unknown",
    title: "Something went wrong. Please try again.",
  };
}

function errorToString(err: unknown): string {
  if (!err) return "";
  if (typeof err === "string") return err;
  if (err instanceof Error) {
    // viem nests reasons across .cause and .details and .shortMessage
    const parts = [err.message, (err as { shortMessage?: string }).shortMessage];
    const cause = (err as { cause?: unknown }).cause;
    if (cause) parts.push(errorToString(cause));
    const details = (err as { details?: string }).details;
    if (details) parts.push(details);
    return parts.filter(Boolean).join(" ");
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

// Faucet links — shown conditionally based on error category
export const BASE_SEPOLIA_USDC_FAUCET =
  "https://faucet.circle.com/"; // Circle's Base Sepolia USDC faucet
export const BASE_SEPOLIA_ETH_FAUCET =
  "https://www.alchemy.com/faucets/base-sepolia";
