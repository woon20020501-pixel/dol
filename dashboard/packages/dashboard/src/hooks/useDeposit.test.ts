import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";

/**
 * useDeposit contract tests — focus on:
 *   (1) needsApproval returning the right boolean across allowance
 *       vs amount boundaries
 *   (2) isReverted firing ONLY for real on-chain reverts (not RPC
 *       errors) via the isRevertError gate
 *   (3) revertedHash pointing to the correct flow (approve vs deposit)
 *
 * All wagmi dependencies are mocked so the hook can be exercised
 * under happy-dom without a provider. translateError, txState, and
 * vault helpers are real.
 */

const mockUseAccount = vi.fn();
const mockUseBalance = vi.fn();
const mockUseReadContract = vi.fn();
const mockUseWriteContract = vi.fn();
const mockUseWaitForReceipt = vi.fn();
const mockUseSwitchChain = vi.fn();
const mockUseChainId = vi.fn();
const mockGetVaultConfig = vi.fn();

vi.mock("wagmi", () => ({
  useAccount: (...a: unknown[]) => mockUseAccount(...a),
  useBalance: (...a: unknown[]) => mockUseBalance(...a),
  useReadContract: (...a: unknown[]) => mockUseReadContract(...a),
  useWriteContract: (...a: unknown[]) => mockUseWriteContract(...a),
  useWaitForTransactionReceipt: (...a: unknown[]) => mockUseWaitForReceipt(...a),
  useSwitchChain: (...a: unknown[]) => mockUseSwitchChain(...a),
  useChainId: (...a: unknown[]) => mockUseChainId(...a),
}));

vi.mock("@/lib/vault", () => ({
  getVaultConfig: () => mockGetVaultConfig(),
  ERC20_ABI: [],
  VAULT_ABI: [],
}));

vi.mock("@/lib/txEvents", () => ({
  emitDolTxConfirmed: vi.fn(),
}));

vi.mock("@/lib/chains", () => ({
  TARGET_CHAIN_ID: 84532,
}));

import { useDeposit } from "./useDeposit";

function defaultWrites() {
  return {
    writeContract: vi.fn(),
    data: undefined,
    error: null,
    isPending: false,
    reset: vi.fn(),
  };
}

function defaultReceipt(overrides: Record<string, unknown> = {}) {
  return {
    data: undefined,
    isLoading: false,
    isSuccess: false,
    isError: false,
    error: null,
    ...overrides,
  };
}

describe("useDeposit", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUseAccount.mockReturnValue({
      address: "0xUser",
      isConnected: true,
    });
    mockUseChainId.mockReturnValue(84532);
    mockUseSwitchChain.mockReturnValue({ switchChain: vi.fn() });
    mockUseBalance.mockReturnValue({
      data: { value: BigInt("1000000000000000"), decimals: 18 }, // 0.001 ETH
    });
    mockGetVaultConfig.mockReturnValue({
      address: "0xVault",
      usdcAddress: "0xUSDC",
    });
    mockUseReadContract.mockReturnValue({
      data: BigInt(5_000_000), // 5 USDC balance OR allowance (depends which read)
      refetch: vi.fn(),
    });
    mockUseWriteContract.mockReturnValue(defaultWrites());
    mockUseWaitForReceipt.mockReturnValue(defaultReceipt());
  });

  describe("needsApproval", () => {
    it("returns false for zero or negative amounts", () => {
      const { result } = renderHook(() => useDeposit());
      expect(result.current.needsApproval(0)).toBe(false);
      expect(result.current.needsApproval(-1)).toBe(false);
    });

    it("returns false when allowance exceeds amount", () => {
      // allowance mocked as 5 USDC (5_000_000) from beforeEach
      const { result } = renderHook(() => useDeposit());
      expect(result.current.needsApproval(1)).toBe(false);
      expect(result.current.needsApproval(4.99)).toBe(false);
    });

    it("returns true when amount exceeds allowance", () => {
      const { result } = renderHook(() => useDeposit());
      expect(result.current.needsApproval(5.01)).toBe(true);
      expect(result.current.needsApproval(100)).toBe(true);
    });
  });

  describe("isReverted gating via isRevertError", () => {
    it("does NOT fire isReverted for RPC/transport errors", () => {
      class HttpRequestError extends Error {
        constructor(msg: string) {
          super(msg);
          this.name = "HttpRequestError";
        }
      }

      // Deposit flow receipt errors with a transport-level failure
      mockUseWriteContract
        .mockReturnValueOnce(defaultWrites()) // approve
        .mockReturnValueOnce({
          ...defaultWrites(),
          data: "0xdeposit_hash" as const,
        }); // deposit
      mockUseWaitForReceipt
        .mockReturnValueOnce(defaultReceipt()) // approve
        .mockReturnValueOnce(
          defaultReceipt({
            isError: true,
            error: new HttpRequestError("502 bad gateway"),
          }),
        );

      const { result } = renderHook(() => useDeposit());

      expect(result.current.isReverted).toBe(false);
      expect(result.current.revertedHash).toBeNull();
    });

    it("fires isReverted for on-chain reverts", () => {
      const revertErr = new Error("InsufficientLiquidity");

      mockUseWriteContract
        .mockReturnValueOnce(defaultWrites())
        .mockReturnValueOnce({
          ...defaultWrites(),
          data: "0xdeposit_hash" as const,
        });
      mockUseWaitForReceipt
        .mockReturnValueOnce(defaultReceipt())
        .mockReturnValueOnce(
          defaultReceipt({
            isError: true,
            error: revertErr,
          }),
        );

      const { result } = renderHook(() => useDeposit());

      expect(result.current.isReverted).toBe(true);
      expect(result.current.revertedHash).toBe("0xdeposit_hash");
    });

    it("points revertedHash at approve flow when approve reverts", () => {
      const revertErr = new Error("SafeERC20: approve from non-zero to non-zero");

      mockUseWriteContract
        .mockReturnValueOnce({
          ...defaultWrites(),
          data: "0xapprove_hash" as const,
        })
        .mockReturnValueOnce(defaultWrites());
      mockUseWaitForReceipt
        .mockReturnValueOnce(
          defaultReceipt({
            isError: true,
            error: revertErr,
          }),
        )
        .mockReturnValueOnce(defaultReceipt());

      const { result } = renderHook(() => useDeposit());

      expect(result.current.isReverted).toBe(true);
      expect(result.current.revertedHash).toBe("0xapprove_hash");
    });
  });

  describe("low-gas pre-flight", () => {
    it("flags hasLowGas when ETH balance is below LOW_GAS_ETH (0.0005)", () => {
      mockUseBalance.mockReturnValue({
        data: { value: BigInt("100000000000000"), decimals: 18 }, // 0.0001 ETH
      });
      const { result } = renderHook(() => useDeposit());
      expect(result.current.hasLowGas).toBe(true);
    });

    it("does not flag when ETH balance is comfortable", () => {
      mockUseBalance.mockReturnValue({
        data: { value: BigInt("10000000000000000"), decimals: 18 }, // 0.01 ETH
      });
      const { result } = renderHook(() => useDeposit());
      expect(result.current.hasLowGas).toBe(false);
    });
  });

  describe("wrong-network detection", () => {
    it("flags isWrongNetwork when connected chain != target", () => {
      mockUseChainId.mockReturnValue(1); // mainnet instead of 84532
      const { result } = renderHook(() => useDeposit());
      expect(result.current.isWrongNetwork).toBe(true);
    });

    it("does not flag when on the target chain", () => {
      mockUseChainId.mockReturnValue(84532);
      const { result } = renderHook(() => useDeposit());
      expect(result.current.isWrongNetwork).toBe(false);
    });
  });
});
