import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

/**
 * useDolWithdraw contract tests.
 *
 * This is the most complex hook in the dashboard: it manages three
 * distinct tx flows (requestRedeem / claimRedeem / instantRedeem),
 * persists pending redeems to localStorage with cross-tab sync, runs
 * an on-chain recovery scanner for orphan requests, and reads
 * cooldownSeconds from the vault contract.
 *
 * The tests below focus on the logic that is exercisable without a
 * live RPC: state merging, reverted-tx gating, cooldown derivation,
 * and localStorage hydration. The event-decoder + recovery-scanner
 * branches require full publicClient mocking and are exercised via
 * the integration e2e flow rather than unit-mocked here.
 */

const mockUseAccount = vi.fn();
const mockUseReadContract = vi.fn();
const mockUseWriteContract = vi.fn();
const mockUseWaitForReceipt = vi.fn();
const mockUsePublicClient = vi.fn();
const mockGetPBondConfig = vi.fn();
const mockGetVaultConfig = vi.fn();
const mockEmit = vi.fn();

vi.mock("wagmi", () => ({
  useAccount: (...a: unknown[]) => mockUseAccount(...a),
  useReadContract: (...a: unknown[]) => mockUseReadContract(...a),
  useWriteContract: (...a: unknown[]) => mockUseWriteContract(...a),
  useWaitForTransactionReceipt: (...a: unknown[]) =>
    mockUseWaitForReceipt(...a),
  usePublicClient: (...a: unknown[]) => mockUsePublicClient(...a),
}));

vi.mock("wagmi/chains", () => ({
  baseSepolia: { id: 84532 },
}));

vi.mock("@/lib/pbond", () => ({
  getPBondConfig: () => mockGetPBondConfig(),
}));

vi.mock("@/lib/vault", () => ({
  getVaultConfig: () => mockGetVaultConfig(),
}));

vi.mock("@/lib/txEvents", () => ({
  emitDolTxConfirmed: (...a: unknown[]) => mockEmit(...a),
}));

import { useDolWithdraw } from "./useDolWithdraw";

const STORAGE_KEY = "dol_pending_redeems";
const USER = "0x1111111111111111111111111111111111111111";
const SENIOR_ADDR = "0x2222222222222222222222222222222222222222";
const VAULT_ADDR = "0x3333333333333333333333333333333333333333";

function defaultWrites(): {
  writeContract: ReturnType<typeof vi.fn>;
  data: `0x${string}` | undefined;
  error: Error | null;
  isPending: boolean;
  reset: ReturnType<typeof vi.fn>;
} {
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

/**
 * The hook calls useWriteContract 3× per render (request / claim /
 * instant) and useWaitForTransactionReceipt 3× per render. React's
 * initial-effect dispatch re-renders the hook, so mockReturnValueOnce
 * gets consumed faster than we'd expect. Instead we use a positional
 * mock — each render, the counter cycles 0→1→2 mapping the three
 * write/receipt slots. Stable across any number of renders.
 */
function mockWritesByPosition(
  request: ReturnType<typeof defaultWrites>,
  claim: ReturnType<typeof defaultWrites>,
  instant: ReturnType<typeof defaultWrites>,
) {
  let n = 0;
  mockUseWriteContract.mockImplementation(() => {
    const which = n % 3;
    n++;
    return which === 0 ? request : which === 1 ? claim : instant;
  });
}

function mockReceiptsByPosition(
  request: ReturnType<typeof defaultReceipt>,
  claim: ReturnType<typeof defaultReceipt>,
  instant: ReturnType<typeof defaultReceipt>,
) {
  let n = 0;
  mockUseWaitForReceipt.mockImplementation(() => {
    const which = n % 3;
    n++;
    return which === 0 ? request : which === 1 ? claim : instant;
  });
}

describe("useDolWithdraw", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mockUseAccount.mockReturnValue({
      address: USER,
      isConnected: true,
    });
    mockGetPBondConfig.mockReturnValue({
      senior: {
        address: SENIOR_ADDR,
        abi: [],
      },
    });
    mockGetVaultConfig.mockReturnValue({
      address: VAULT_ADDR,
      abi: [],
    });
    // Default cooldown read: 1800 seconds (30 min)
    mockUseReadContract.mockReturnValue({
      data: BigInt(1800),
    });
    mockUseWriteContract.mockReturnValue(defaultWrites());
    mockUseWaitForReceipt.mockReturnValue(defaultReceipt());
    mockUsePublicClient.mockReturnValue({
      readContract: vi.fn().mockResolvedValue(BigInt(0)),
    });
  });

  describe("cooldown derivation", () => {
    it("uses on-chain cooldownSeconds when the read succeeds", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(1800) });
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.cooldownMs).toBe(1_800_000);
    });

    it("falls back to 30-minute default when on-chain read returns non-bigint", () => {
      mockUseReadContract.mockReturnValue({ data: undefined });
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.cooldownMs).toBe(30 * 60 * 1000);
    });

    it("honors an on-chain value different from the fallback", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(3600) }); // 1 hour
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.cooldownMs).toBe(3_600_000);
    });
  });

  describe("isClaimable / cooldownRemaining", () => {
    it("isClaimable returns false before cooldown elapses", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(1800) });
      const { result } = renderHook(() => useDolWithdraw());
      const req = {
        requestId: "1",
        shares: 10,
        requestedAt: Date.now() - 60_000, // 1 min ago
      };
      expect(result.current.isClaimable(req)).toBe(false);
    });

    it("isClaimable returns true after cooldown elapses", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(1800) });
      const { result } = renderHook(() => useDolWithdraw());
      const req = {
        requestId: "1",
        shares: 10,
        requestedAt: Date.now() - 2_000_000, // ~33 min ago
      };
      expect(result.current.isClaimable(req)).toBe(true);
    });

    it("cooldownRemaining clamps to 0 once elapsed", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(1800) });
      const { result } = renderHook(() => useDolWithdraw());
      const req = {
        requestId: "1",
        shares: 10,
        requestedAt: Date.now() - 3_600_000, // 1 hour ago
      };
      expect(result.current.cooldownRemaining(req)).toBe(0);
    });

    it("cooldownRemaining returns positive ms when pending", () => {
      mockUseReadContract.mockReturnValue({ data: BigInt(1800) });
      const { result } = renderHook(() => useDolWithdraw());
      const req = {
        requestId: "1",
        shares: 10,
        requestedAt: Date.now() - 60_000, // 1 min ago → ~29 min remaining
      };
      const remaining = result.current.cooldownRemaining(req);
      expect(remaining).toBeGreaterThan(28 * 60_000);
      expect(remaining).toBeLessThan(30 * 60_000);
    });
  });

  describe("pending list localStorage hydration", () => {
    it("loads a valid pending list on mount", () => {
      const reqs = [
        { requestId: "42", shares: 100, requestedAt: Date.now() - 5000 },
      ];
      localStorage.setItem(`${STORAGE_KEY}_${USER}`, JSON.stringify(reqs));

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toHaveLength(1);
      expect(result.current.pending[0].requestId).toBe("42");
      expect(result.current.pending[0].shares).toBe(100);
    });

    it("returns empty list when localStorage has no key for this user", () => {
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toEqual([]);
    });

    it("returns empty list when stored value is malformed JSON", () => {
      localStorage.setItem(`${STORAGE_KEY}_${USER}`, "{not valid json");
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toEqual([]);
    });

    it("discards entries that fail shape validation", () => {
      // Mix of one valid entry and two invalid ones (missing shares / bad types)
      const mixed = [
        { requestId: "42", shares: 100, requestedAt: Date.now() },
        { requestId: "43", shares: "not a number", requestedAt: Date.now() },
        { requestId: null, shares: 50, requestedAt: Date.now() },
      ];
      localStorage.setItem(`${STORAGE_KEY}_${USER}`, JSON.stringify(mixed));

      const { result } = renderHook(() => useDolWithdraw());
      // Only the well-formed entry survives guard validation
      expect(result.current.pending).toHaveLength(1);
      expect(result.current.pending[0].requestId).toBe("42");
    });

    it("returns empty list when wallet is disconnected", () => {
      mockUseAccount.mockReturnValue({
        address: undefined,
        isConnected: false,
      });
      localStorage.setItem(
        `${STORAGE_KEY}_${USER}`,
        JSON.stringify([
          { requestId: "42", shares: 100, requestedAt: Date.now() },
        ]),
      );
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toEqual([]);
    });
  });

  describe("cross-tab storage sync", () => {
    it("reacts to a `storage` event for the same user's key", () => {
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toEqual([]);

      const newEntry = {
        requestId: "99",
        shares: 5,
        requestedAt: Date.now(),
      };
      localStorage.setItem(
        `${STORAGE_KEY}_${USER}`,
        JSON.stringify([newEntry]),
      );

      act(() => {
        // Fire a synthetic storage event as if another tab wrote to it.
        window.dispatchEvent(
          new StorageEvent("storage", {
            key: `${STORAGE_KEY}_${USER}`,
            newValue: JSON.stringify([newEntry]),
          }),
        );
      });

      expect(result.current.pending).toHaveLength(1);
      expect(result.current.pending[0].requestId).toBe("99");
    });

    it("ignores storage events for unrelated keys", () => {
      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.pending).toEqual([]);

      act(() => {
        window.dispatchEvent(
          new StorageEvent("storage", {
            key: "some_other_key",
            newValue: "irrelevant",
          }),
        );
      });

      expect(result.current.pending).toEqual([]);
    });
  });

  describe("isReverted gating via isRevertError", () => {
    it("does NOT fire isReverted for viem-style RPC errors", () => {
      class HttpRequestError extends Error {
        constructor(msg: string) {
          super(msg);
          this.name = "HttpRequestError";
        }
      }

      mockWritesByPosition(
        { ...defaultWrites(), data: "0xrequest_hash" as const },
        defaultWrites(),
        defaultWrites(),
      );
      mockReceiptsByPosition(
        defaultReceipt({
          isError: true,
          error: new HttpRequestError("502 bad gateway"),
        }),
        defaultReceipt(),
        defaultReceipt(),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.isReverted).toBe(false);
      expect(result.current.revertedHash).toBeNull();
    });

    it("fires isReverted + revertedHash for on-chain reverts on request flow", () => {
      mockWritesByPosition(
        { ...defaultWrites(), data: "0xrequest_hash" as const },
        defaultWrites(),
        defaultWrites(),
      );
      mockReceiptsByPosition(
        defaultReceipt({
          isError: true,
          error: new Error("InsufficientShares"),
        }),
        defaultReceipt(),
        defaultReceipt(),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.isReverted).toBe(true);
      expect(result.current.revertedHash).toBe("0xrequest_hash");
    });

    it("picks claim flow hash when claim reverts (not request)", () => {
      mockWritesByPosition(
        defaultWrites(),
        { ...defaultWrites(), data: "0xclaim_hash" as const },
        defaultWrites(),
      );
      mockReceiptsByPosition(
        defaultReceipt(),
        defaultReceipt({
          isError: true,
          error: new Error("AlreadyClaimed"),
        }),
        defaultReceipt(),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.isReverted).toBe(true);
      expect(result.current.revertedHash).toBe("0xclaim_hash");
    });

    it("picks instant flow hash when instant reverts", () => {
      mockWritesByPosition(
        defaultWrites(),
        defaultWrites(),
        { ...defaultWrites(), data: "0xinstant_hash" as const },
      );
      mockReceiptsByPosition(
        defaultReceipt(),
        defaultReceipt(),
        defaultReceipt({
          isError: true,
          error: new Error("InsufficientLiquidity"),
        }),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.isReverted).toBe(true);
      expect(result.current.revertedHash).toBe("0xinstant_hash");
    });
  });

  describe("error merging", () => {
    it("writeContract error takes precedence over receipt error", () => {
      const walletErr = new Error("User rejected");
      const receiptErr = new Error("This should be masked");

      mockWritesByPosition(
        { ...defaultWrites(), error: walletErr, data: "0xhash" as const },
        defaultWrites(),
        defaultWrites(),
      );
      mockReceiptsByPosition(
        defaultReceipt({ isError: true, error: receiptErr }),
        defaultReceipt(),
        defaultReceipt(),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.requestError).toBe(walletErr);
    });

    it("receipt error surfaces when writeContract error is null", () => {
      const receiptErr = new Error("InsufficientBalance");

      mockWritesByPosition(
        { ...defaultWrites(), data: "0xhash" as const },
        defaultWrites(),
        defaultWrites(),
      );
      mockReceiptsByPosition(
        defaultReceipt({ isError: true, error: receiptErr }),
        defaultReceipt(),
        defaultReceipt(),
      );

      const { result } = renderHook(() => useDolWithdraw());
      expect(result.current.requestError).toBe(receiptErr);
    });
  });
});
