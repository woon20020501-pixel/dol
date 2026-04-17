"use client";

import { useEffect } from "react";
import { useReadContracts, useAccount } from "wagmi";
import { getVaultConfig, MOONWELL_ABI } from "@/lib/vault";
import { formatUnits } from "viem";
import { onDolStateShouldRefresh } from "@/lib/txEvents";

const USDC_DECIMALS = 6;
const SHARE_DECIMALS = 6;
// sharePrice() returns a 1e18-base ratio (standard ERC-4626 price unit),
// NOT a USDC-denominated amount. Empty vault returns exactly 1e18.
const SHARE_PRICE_DECIMALS = 18;

/**
 * Reads vault contract state: totalAssets, sharePrice, and user balance.
 * Returns null values when contract is not deployed or wallet not connected.
 */
export function useVaultReads() {
  const vaultConfig = getVaultConfig();
  const { address: userAddress, isConnected } = useAccount();

  const contractBase = vaultConfig
    ? { address: vaultConfig.address, abi: vaultConfig.abi }
    : undefined;

  const treasuryVault = vaultConfig?.treasuryVault ?? null;

  // Build contract list. Treasury read is appended last when available so
  // its index is deterministic regardless of whether userAddress is set.
  const baseContracts = contractBase
    ? [
        { ...contractBase, functionName: "totalAssets" } as const,
        { ...contractBase, functionName: "sharePrice" } as const,
      ]
    : [];

  const userContracts =
    contractBase && userAddress
      ? [
          {
            ...contractBase,
            functionName: "balanceOf",
            args: [userAddress],
          } as const,
        ]
      : [];

  const treasuryContracts =
    contractBase && treasuryVault
      ? [
          {
            address: treasuryVault,
            abi: MOONWELL_ABI,
            functionName: "balanceOfUnderlying",
            args: [contractBase.address],
          } as const,
        ]
      : [];

  const userIndex = userAddress ? 2 : -1;
  const treasuryIndex = userAddress ? 3 : 2;

  const { data, isLoading, isError, error, refetch } = useReadContracts({
    contracts: contractBase
      ? [...baseContracts, ...userContracts, ...treasuryContracts]
      : [],
    query: {
      enabled: !!contractBase,
      refetchInterval: 10_000, // re-read every 10s
    },
  });

  // Refetch on every dol tx confirm + tab visibility so the operator
  // dashboard's vault totals stay in sync with deposits/withdraws that
  // happen on the landing page or /deposit flow.
  useEffect(() => {
    const unsub = onDolStateShouldRefresh(() => refetch());
    return unsub;
  }, [refetch]);

  if (!contractBase) {
    // Shape parity with the main return below — consumers destructure
    // `error` unconditionally, so the fallback branch must expose the
    // same keys (null-valued) instead of omitting them. Prevents
    // `vault.error` becoming `undefined` pre-deploy.
    return {
      deployed: false,
      isLoading: false,
      isError: false,
      error: null,
      totalAssets: null,
      sharePrice: null,
      userShares: null,
      userAssetsValue: null,
      treasuryAssets: null,
      marginAssets: null,
      treasuryShare: null,
      marginShare: null,
      treasuryConnected: false,
      allocation: { treasuryBps: 3000, marginBps: 7000 },
      refetch,
    };
  }

  const totalAssetsRaw = data?.[0]?.result as bigint | undefined;
  const sharePriceRaw = data?.[1]?.result as bigint | undefined;
  const userSharesRaw = userIndex >= 0 ? (data?.[userIndex]?.result as bigint | undefined) : undefined;
  const treasuryRaw = treasuryVault
    ? (data?.[treasuryIndex]?.result as bigint | undefined)
    : undefined;

  const totalAssets = totalAssetsRaw !== undefined
    ? Number(formatUnits(totalAssetsRaw, USDC_DECIMALS))
    : null;

  const sharePrice = sharePriceRaw !== undefined
    ? Number(formatUnits(sharePriceRaw, SHARE_PRICE_DECIMALS))
    : null;

  const userShares =
    isConnected && userSharesRaw !== undefined
      ? Number(formatUnits(userSharesRaw, SHARE_DECIMALS))
      : null;

  const userAssetsValue =
    userShares !== null && sharePrice !== null
      ? userShares * sharePrice
      : null;

  // Treasury balance: real on-chain value if treasuryVault is wired,
  // otherwise derive a synthetic split from the configured allocation
  // so the AllocationBar still tells a story pre-deploy.
  //
  // Past this point `contractBase` is narrowed non-null by the early
  // return above, but TS can't correlate that narrowing back to
  // `vaultConfig` (different binding). Fall back to the default
  // allocation if vaultConfig somehow became null between checks —
  // semantically impossible since getVaultConfig() is pure and
  // idempotent, but the fallback keeps us free of non-null assertions.
  const allocation = vaultConfig?.allocation ?? {
    treasuryBps: 3000,
    marginBps: 7000,
  };
  let treasuryAssets: number | null = null;
  let marginAssets: number | null = null;
  let treasuryConnected = false;

  if (treasuryRaw !== undefined) {
    treasuryAssets = Number(formatUnits(treasuryRaw, USDC_DECIMALS));
    treasuryConnected = true;
    if (totalAssets !== null) {
      marginAssets = Math.max(0, totalAssets - treasuryAssets);
    }
  } else if (totalAssets !== null) {
    // Synthetic: split totalAssets by configured bps
    treasuryAssets = (totalAssets * allocation.treasuryBps) / 10_000;
    marginAssets = (totalAssets * allocation.marginBps) / 10_000;
  }

  const denom = (treasuryAssets ?? 0) + (marginAssets ?? 0);
  const treasuryShare = denom > 0 ? (treasuryAssets ?? 0) / denom : null;
  const marginShare = denom > 0 ? (marginAssets ?? 0) / denom : null;

  return {
    deployed: true,
    isLoading,
    isError,
    error,
    totalAssets,
    sharePrice,
    userShares,
    userAssetsValue,
    treasuryAssets,
    marginAssets,
    treasuryShare,
    marginShare,
    treasuryConnected,
    allocation,
    refetch,
  };
}
