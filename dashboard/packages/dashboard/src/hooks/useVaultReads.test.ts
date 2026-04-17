import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";

/**
 * useVaultReads contract tests.
 *
 * We mock both wagmi and the vault-config getter so the hook can be
 * exercised in isolation under happy-dom — no RPC, no WagmiProvider
 * needed. The shape parity check is the critical one: consumers
 * destructure `vault.error` (and friends) unconditionally, so the
 * pre-deploy fallback branch MUST return identical keys as the main
 * branch. Missing a key there caused `undefined.whatever` crashes in
 * the past.
 */

const mockUseAccount = vi.fn();
const mockUseReadContracts = vi.fn();
const mockGetVaultConfig = vi.fn();

vi.mock("wagmi", () => ({
  useAccount: (...args: unknown[]) => mockUseAccount(...args),
  useReadContracts: (...args: unknown[]) => mockUseReadContracts(...args),
}));

vi.mock("@/lib/vault", () => ({
  getVaultConfig: () => mockGetVaultConfig(),
  MOONWELL_ABI: [],
}));

vi.mock("@/lib/txEvents", () => ({
  onDolStateShouldRefresh: () => () => undefined,
}));

import { useVaultReads } from "./useVaultReads";

const EXPECTED_KEYS = [
  "deployed",
  "isLoading",
  "isError",
  "error",
  "totalAssets",
  "sharePrice",
  "userShares",
  "userAssetsValue",
  "treasuryAssets",
  "marginAssets",
  "treasuryShare",
  "marginShare",
  "treasuryConnected",
  "allocation",
  "refetch",
] as const;

describe("useVaultReads", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("shape parity", () => {
    it("fallback branch (contract not deployed) exposes every key", () => {
      mockGetVaultConfig.mockReturnValue(null);
      mockUseAccount.mockReturnValue({
        address: undefined,
        isConnected: false,
      });
      mockUseReadContracts.mockReturnValue({
        data: undefined,
        isLoading: false,
        isError: false,
        error: null,
        refetch: vi.fn(),
      });

      const { result } = renderHook(() => useVaultReads());

      for (const key of EXPECTED_KEYS) {
        expect(result.current).toHaveProperty(key);
      }
      // All data values null; structural guarantees for consumers
      expect(result.current.deployed).toBe(false);
      expect(result.current.error).toBeNull();
      expect(result.current.totalAssets).toBeNull();
      expect(result.current.refetch).toBeTypeOf("function");
    });

    it("main branch (contract deployed, wallet connected) exposes every key", () => {
      mockGetVaultConfig.mockReturnValue({
        address: "0xVault",
        abi: [],
        treasuryVault: null,
        allocation: { treasuryBps: 3000, marginBps: 7000 },
      });
      mockUseAccount.mockReturnValue({
        address: "0xUser",
        isConnected: true,
      });
      // totalAssets = 1_000_000 USDC (6 decimals) = 1 USDC
      // sharePrice = 1e18 (ERC-4626 base) = 1.0
      // userShares = 500_000 (6 decimals) = 0.5
      mockUseReadContracts.mockReturnValue({
        data: [
          { result: BigInt(1_000_000), status: "success" },
          { result: BigInt("1000000000000000000"), status: "success" },
          { result: BigInt(500_000), status: "success" },
        ],
        isLoading: false,
        isError: false,
        error: null,
        refetch: vi.fn(),
      });

      const { result } = renderHook(() => useVaultReads());

      for (const key of EXPECTED_KEYS) {
        expect(result.current).toHaveProperty(key);
      }
      expect(result.current.deployed).toBe(true);
      expect(result.current.totalAssets).toBe(1);
      expect(result.current.sharePrice).toBe(1);
      expect(result.current.userShares).toBe(0.5);
      expect(result.current.userAssetsValue).toBe(0.5);
    });
  });

  describe("synthetic allocation split when treasury vault not wired", () => {
    it("splits totalAssets by allocation bps", () => {
      mockGetVaultConfig.mockReturnValue({
        address: "0xVault",
        abi: [],
        treasuryVault: null,
        allocation: { treasuryBps: 3000, marginBps: 7000 }, // 30% / 70%
      });
      mockUseAccount.mockReturnValue({
        address: undefined,
        isConnected: false,
      });
      mockUseReadContracts.mockReturnValue({
        data: [
          { result: BigInt(10_000_000), status: "success" }, // 10 USDC
          { result: BigInt("1000000000000000000"), status: "success" },
        ],
        isLoading: false,
        isError: false,
        error: null,
        refetch: vi.fn(),
      });

      const { result } = renderHook(() => useVaultReads());

      expect(result.current.treasuryConnected).toBe(false);
      expect(result.current.treasuryAssets).toBeCloseTo(3); // 30% of 10
      expect(result.current.marginAssets).toBeCloseTo(7); // 70% of 10
      expect(result.current.treasuryShare).toBeCloseTo(0.3);
      expect(result.current.marginShare).toBeCloseTo(0.7);
    });
  });

  describe("error state", () => {
    it("surfaces isError + error from underlying useReadContracts", () => {
      const boom = new Error("HTTP 503");
      mockGetVaultConfig.mockReturnValue({
        address: "0xVault",
        abi: [],
        treasuryVault: null,
        allocation: { treasuryBps: 3000, marginBps: 7000 },
      });
      mockUseAccount.mockReturnValue({
        address: undefined,
        isConnected: false,
      });
      mockUseReadContracts.mockReturnValue({
        data: undefined,
        isLoading: false,
        isError: true,
        error: boom,
        refetch: vi.fn(),
      });

      const { result } = renderHook(() => useVaultReads());

      expect(result.current.isError).toBe(true);
      expect(result.current.error).toBe(boom);
    });
  });
});
