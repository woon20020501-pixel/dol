import { describe, it, expect } from "vitest";
import { translateError, type ErrorCategory } from "./errors";

/**
 * Error translator contract tests. Covers every category the UI branches
 * on + the precedence order of overlapping patterns. If these fail, a
 * user-visible toast somewhere will regress to the raw wagmi string.
 */

function cat(e: unknown): ErrorCategory {
  return translateError(e).category;
}

describe("translateError", () => {
  describe("user_rejected (highest precedence — wallet popup dismissed)", () => {
    it("matches viem UserRejectedRequestError", () => {
      expect(cat(new Error("UserRejectedRequestError: user rejected")))
        .toBe("user_rejected");
    });

    it("matches MetaMask legacy 'user denied'", () => {
      expect(cat(new Error("MetaMask Tx Signature: User denied transaction signature.")))
        .toBe("user_rejected");
    });

    it("matches 'rejected the request'", () => {
      expect(cat(new Error("User rejected the request.")))
        .toBe("user_rejected");
    });

    it("returns neutral copy (NOT 'error')", () => {
      // UX invariant: user-rejected must be NEUTRAL, not a red "error"
      // The toast consumer uses this category to pick toast() vs toast.error().
      const r = translateError(new Error("user rejected"));
      expect(r.title.toLowerCase()).not.toContain("error");
      expect(r.title.toLowerCase()).not.toContain("failed");
    });
  });

  describe("insufficient_usdc", () => {
    it("matches ERC20 transfer amount exceeds balance", () => {
      expect(cat(new Error("ERC20: transfer amount exceeds balance")))
        .toBe("insufficient_usdc");
    });

    it("matches 'insufficient balance'", () => {
      expect(cat(new Error("Transfer failed: insufficient balance")))
        .toBe("insufficient_usdc");
    });

    it("matches 'exceeds allowance'", () => {
      expect(cat(new Error("ERC20: transfer amount exceeds allowance")))
        .toBe("insufficient_usdc");
    });
  });

  describe("insufficient_eth", () => {
    it("matches 'insufficient funds for gas'", () => {
      expect(cat(new Error("insufficient funds for gas * price + value")))
        .toBe("insufficient_eth");
    });

    it("matches 'out of gas'", () => {
      expect(cat(new Error("execution reverted: out of gas")))
        .toBe("insufficient_eth"); // gas precedence > revert
    });

    it("matches 'intrinsic gas too low'", () => {
      expect(cat(new Error("intrinsic gas too low")))
        .toBe("insufficient_eth");
    });
  });

  describe("wrong_network", () => {
    it("matches chain mismatch", () => {
      expect(cat(new Error("Chain mismatch: expected 84532, got 1")))
        .toBe("wrong_network");
    });

    it("matches 'does not match the target chain'", () => {
      expect(cat(new Error("The current chain does not match the target chain.")))
        .toBe("wrong_network");
    });
  });

  describe("network_glitch", () => {
    it("matches 'failed to fetch'", () => {
      expect(cat(new Error("TypeError: Failed to fetch")))
        .toBe("network_glitch");
    });

    it("matches RPC timeouts", () => {
      expect(cat(new Error("RPC timeout while calling eth_call")))
        .toBe("network_glitch");
    });

    it("matches ECONNRESET", () => {
      expect(cat(new Error("ECONNRESET")))
        .toBe("network_glitch");
    });
  });

  describe("contract_revert (catch-all for execution reverted)", () => {
    it("matches 'execution reverted' with unknown reason", () => {
      expect(cat(new Error("execution reverted: UnknownCustomError()")))
        .toBe("contract_revert");
    });

    it("matches 'ContractFunctionExecutionError'", () => {
      expect(cat(new Error("ContractFunctionExecutionError: The contract function reverted")))
        .toBe("contract_revert");
    });

    it("NEVER says 'your money is safe' — contradicts at-risk disclaimer", () => {
      // Regression guard: legal review rejected reassuring copy. Tx reverts
      // are atomic so funds didn't move, but the UI must not imply safety
      // guarantees elsewhere.
      const r = translateError(new Error("execution reverted"));
      const combined = (r.title + " " + (r.description ?? "")).toLowerCase();
      expect(combined).not.toContain("safe");
      expect(combined).not.toContain("guaranteed");
    });
  });

  describe("unknown fallback", () => {
    it("falls through on empty input", () => {
      expect(cat(null)).toBe("unknown");
      expect(cat(undefined)).toBe("unknown");
      expect(cat("")).toBe("unknown");
    });

    it("falls through on genuinely unrecognized errors", () => {
      expect(cat(new Error("something we never saw before")))
        .toBe("unknown");
    });
  });

  describe("nested viem error shapes", () => {
    it("unwraps .cause chain", () => {
      const inner = new Error("user rejected the request");
      const outer = new Error("TransactionExecutionError");
      (outer as Error & { cause?: unknown }).cause = inner;
      expect(cat(outer)).toBe("user_rejected");
    });

    it("reads viem .shortMessage field", () => {
      const err = new Error("long noisy stack") as Error & {
        shortMessage?: string;
      };
      err.shortMessage = "User rejected the request.";
      expect(cat(err)).toBe("user_rejected");
    });

    it("reads viem .details field", () => {
      const err = new Error("generic wrap") as Error & { details?: string };
      err.details = "insufficient funds for gas * price";
      expect(cat(err)).toBe("insufficient_eth");
    });
  });

  describe("returned UserError shape", () => {
    it("always has title", () => {
      const r = translateError(new Error("anything"));
      expect(r.title).toBeTruthy();
      expect(typeof r.title).toBe("string");
    });

    it("contract_revert carries a description with reassurance phrasing", () => {
      const r = translateError(new Error("execution reverted"));
      expect(r.description).toBeTruthy();
      // Guard with a conditional so TS narrows r.description without a
      // non-null assertion; the toBeTruthy above is the functional assert.
      if (r.description) {
        expect(r.description.toLowerCase()).toContain("no dol was bought");
      }
    });
  });
});
