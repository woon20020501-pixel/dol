import { describe, it, expect } from "vitest";
import {
  HttpRequestError,
  WaitForTransactionReceiptTimeoutError,
  TransactionReceiptNotFoundError,
  TransactionExecutionError,
  ContractFunctionExecutionError,
  ContractFunctionRevertedError,
  TimeoutError,
  BaseError,
} from "viem";
import { isRevertError } from "./txState";

// viem's TransactionExecutionError / ContractFunctionExecutionError
// constructors require a BaseError-shaped cause (not a plain Error).
// We build a minimal BaseError for test-only purposes — the cause is
// opaque to the isRevertError heuristic, which only reads the
// outer error's .name.
function buildBaseErrorCause(shortMessage: string): BaseError {
  return new BaseError(shortMessage);
}

/**
 * Integration test: isRevertError against REAL viem error classes.
 *
 * The plain unit test in txState.test.ts uses synthetic Error subclasses
 * to probe the heuristic. This file locks in the contract against the
 * actual viem error shapes that wagmi's useWaitForTransactionReceipt
 * surfaces. If viem adds a new named error class in a future minor
 * version that we haven't accounted for, this file is where the
 * regression would surface.
 *
 * Packages are pinned in package.json (wagmi 3.6.1 / viem 2.47.11 /
 * @privy-io/wagmi 4.0.4) so a dependabot bump is required to move
 * these versions — at which point this file must be re-audited to
 * confirm the heuristic still holds.
 *
 * See wagmi core 2.22.1's actions/waitForTransactionReceipt.ts for
 * the canonical revert-path throw:
 *     if (receipt.status === 'reverted') throw new Error(reason);
 * Any deviation from this shape would require updating isRevertError.
 */

describe("isRevertError — viem error class integration", () => {
  it("viem RPC transport errors are NOT treated as reverts", () => {
    const http = new HttpRequestError({
      url: "https://sepolia.base.org",
      status: 500,
      body: { method: "eth_getTransactionReceipt", params: [] },
    });
    expect(http.name).toBe("HttpRequestError");
    expect(isRevertError(http)).toBe(false);
  });

  it("viem timeout errors are NOT treated as reverts", () => {
    const waitTimeout = new WaitForTransactionReceiptTimeoutError({
      hash: "0xdeadbeef",
    });
    expect(waitTimeout.name).toBe("WaitForTransactionReceiptTimeoutError");
    expect(isRevertError(waitTimeout)).toBe(false);

    const generic = new TimeoutError({
      body: { method: "eth_getTransactionReceipt" },
      url: "https://sepolia.base.org",
    });
    expect(generic.name).toBe("TimeoutError");
    expect(isRevertError(generic)).toBe(false);
  });

  it("viem TransactionReceiptNotFoundError is NOT treated as revert", () => {
    const notFound = new TransactionReceiptNotFoundError({
      hash: "0xdeadbeef",
    });
    expect(notFound.name).toBe("TransactionReceiptNotFoundError");
    expect(isRevertError(notFound)).toBe(false);
  });

  it("viem TransactionExecutionError (pre-send failure) is NOT a revert", () => {
    const txExec = new TransactionExecutionError(
      buildBaseErrorCause("user rejected"),
      { account: null },
    );
    expect(txExec.name).toBe("TransactionExecutionError");
    expect(isRevertError(txExec)).toBe(false);
  });

  it("viem ContractFunctionExecutionError is NOT a revert", () => {
    // This is thrown by contract write/read operations, distinct from
    // the revert-detection path. The name is unique so the heuristic
    // correctly ignores it.
    const contractExec = new ContractFunctionExecutionError(
      buildBaseErrorCause("inner"),
      { abi: [], functionName: "transfer" },
    );
    expect(contractExec.name).toBe("ContractFunctionExecutionError");
    expect(isRevertError(contractExec)).toBe(false);
  });

  it("viem ContractFunctionRevertedError is NOT treated as revert by OUR heuristic", () => {
    // IMPORTANT: This is a subtle point. viem has its own
    // ContractFunctionRevertedError class for read-call reverts, but
    // wagmi's waitForTransactionReceipt does NOT throw this — it
    // throws a plain `new Error(reason)`. Our isRevertError is
    // narrowly designed to detect the plain-Error throw from wagmi's
    // receipt flow. If any consumer ends up passing a viem
    // ContractFunctionRevertedError, we correctly return false
    // (not our code path).
    const cfre = new ContractFunctionRevertedError({
      abi: [],
      functionName: "transfer",
      data: "0x",
    });
    expect(cfre.name).toBe("ContractFunctionRevertedError");
    expect(isRevertError(cfre)).toBe(false);
  });

  it("viem BaseError (generic wrapper) is NOT a revert", () => {
    const base = new BaseError("something went wrong");
    expect(base.name).toBe("BaseError");
    expect(isRevertError(base)).toBe(false);
  });

  it("plain Error constructed like wagmi's revert path IS a revert", () => {
    // This mirrors the exact call in @wagmi/core 2.22.1's
    // waitForTransactionReceipt.ts:83 — `throw new Error(reason)`
    // where `reason` is the decoded revert string or the literal
    // "unknown reason" fallback.
    const decodedReason = new Error("InsufficientLiquidity");
    expect(decodedReason.name).toBe("Error");
    expect(isRevertError(decodedReason)).toBe(true);

    const unknownReason = new Error("unknown reason");
    expect(isRevertError(unknownReason)).toBe(true);

    const abiErrorString = new Error(
      "ERC20: transfer amount exceeds balance",
    );
    expect(isRevertError(abiErrorString)).toBe(true);
  });
});
