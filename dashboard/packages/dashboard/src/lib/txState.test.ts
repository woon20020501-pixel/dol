import { describe, it, expect } from "vitest";
import {
  pickRevertedHash,
  anyReverted,
  mergeTxError,
  isRevertError,
} from "./txState";

/**
 * Contract tests for the tx-state aggregation helpers used by the
 * deposit/withdraw hooks. These cover the reverted-tx UI affordance
 * (T3 / C2) — a regression here would silently drop the basescan
 * deep-link from toasts and error banners.
 */

describe("pickRevertedHash", () => {
  it("returns null when no flow is reverted", () => {
    const flows = [
      { isReverted: false, hash: "0xabc" as const },
      { isReverted: false, hash: undefined },
    ];
    expect(pickRevertedHash(flows)).toBeNull();
  });

  it("returns the hash of the first reverted flow", () => {
    const flows = [
      { isReverted: false, hash: "0xaaa" as const },
      { isReverted: true, hash: "0xbbb" as const },
      { isReverted: true, hash: "0xccc" as const },
    ];
    expect(pickRevertedHash(flows)).toBe("0xbbb");
  });

  it("skips reverted flows that have no hash and keeps searching", () => {
    // A flow can briefly be isReverted=true but hash=undefined during
    // the race window between writeContract rejecting and the reset
    // effect clearing state. The helper must skip it and prefer a
    // reverted flow that actually has a linkable hash.
    const flows = [
      { isReverted: true, hash: undefined },
      { isReverted: true, hash: "0xddd" as const },
    ];
    expect(pickRevertedHash(flows)).toBe("0xddd");
  });

  it("returns null if every reverted flow has no hash", () => {
    const flows = [
      { isReverted: true, hash: undefined },
      { isReverted: true, hash: undefined },
    ];
    expect(pickRevertedHash(flows)).toBeNull();
  });

  it("handles empty flow list", () => {
    expect(pickRevertedHash([])).toBeNull();
  });
});

describe("anyReverted", () => {
  it("returns false for all-healthy flows", () => {
    expect(
      anyReverted([
        { isReverted: false, hash: "0x1" as const },
        { isReverted: false, hash: undefined },
      ]),
    ).toBe(false);
  });

  it("returns true if at least one flow is reverted", () => {
    expect(
      anyReverted([
        { isReverted: false, hash: "0x1" as const },
        { isReverted: true, hash: "0x2" as const },
      ]),
    ).toBe(true);
  });

  it("returns false for empty list", () => {
    expect(anyReverted([])).toBe(false);
  });
});

describe("mergeTxError", () => {
  it("returns writeErr when both are set (writeErr has precedence)", () => {
    const writeErr = new Error("User rejected");
    const receiptErr = new Error("Reverted on-chain");
    expect(mergeTxError(writeErr, receiptErr)).toBe(writeErr);
  });

  it("returns writeErr when only writeErr is set", () => {
    const writeErr = new Error("User rejected");
    expect(mergeTxError(writeErr, null)).toBe(writeErr);
  });

  it("returns receiptErr when only receiptErr is set", () => {
    const receiptErr = new Error("Reverted on-chain");
    expect(mergeTxError(null, receiptErr)).toBe(receiptErr);
  });

  it("returns null when neither error is set", () => {
    expect(mergeTxError(null, null)).toBeNull();
  });

  it("treats undefined like null", () => {
    // wagmi's error field may be `undefined` (not `null`) when no
    // error has fired. The helper must accept both without crashing.
    expect(mergeTxError(undefined, undefined)).toBeNull();
    const receiptErr = new Error("revert");
    expect(mergeTxError(undefined, receiptErr)).toBe(receiptErr);
  });
});

describe("isRevertError", () => {
  it("returns false for null / undefined / non-Error inputs", () => {
    expect(isRevertError(null)).toBe(false);
    expect(isRevertError(undefined)).toBe(false);
    expect(isRevertError("revert")).toBe(false);
    expect(isRevertError({ message: "revert" })).toBe(false);
    expect(isRevertError(42)).toBe(false);
  });

  it("returns true for plain Error with a decoded revert reason", () => {
    // wagmi core 2.22.1 throws `new Error(reason)` on receipt.status
    // === 'reverted'; .name defaults to "Error" and .message is the
    // decoded revert reason.
    expect(isRevertError(new Error("InsufficientLiquidity"))).toBe(true);
    expect(isRevertError(new Error("ERC20: insufficient allowance"))).toBe(
      true,
    );
    expect(isRevertError(new Error("unknown reason"))).toBe(true);
  });

  it("returns false for named viem / wagmi RPC error subclasses", () => {
    // These are the shapes viem throws for transport-level failures.
    // Mislabeling them as reverts would send users chasing on-chain
    // debugging for what is actually a flaky RPC.
    class HttpRequestError extends Error {
      constructor(msg: string) {
        super(msg);
        this.name = "HttpRequestError";
      }
    }
    class WaitForTransactionReceiptTimeoutError extends Error {
      constructor(msg: string) {
        super(msg);
        this.name = "WaitForTransactionReceiptTimeoutError";
      }
    }
    class TransactionReceiptNotFoundError extends Error {
      constructor(msg: string) {
        super(msg);
        this.name = "TransactionReceiptNotFoundError";
      }
    }
    expect(isRevertError(new HttpRequestError("502 bad gateway"))).toBe(
      false,
    );
    expect(
      isRevertError(
        new WaitForTransactionReceiptTimeoutError("timed out after 30s"),
      ),
    ).toBe(false);
    expect(
      isRevertError(
        new TransactionReceiptNotFoundError("receipt not found for 0xabc"),
      ),
    ).toBe(false);
  });

  it("returns false for Error with empty message", () => {
    // A plain `new Error()` with no message is ambiguous — possibly
    // a revert with no reason, possibly some other code path. We bias
    // toward not labeling ambiguous errors as reverts.
    expect(isRevertError(new Error(""))).toBe(false);
    expect(isRevertError(new Error())).toBe(false);
  });
});
