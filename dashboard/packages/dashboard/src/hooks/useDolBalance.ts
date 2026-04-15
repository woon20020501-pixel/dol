"use client";

import { useEffect } from "react";
import { useAccount, useReadContract } from "wagmi";
import { baseSepolia } from "wagmi/chains";
import { type Abi } from "viem";
import { getPBondConfig } from "@/lib/pbond";
import { onDolStateShouldRefresh } from "@/lib/txEvents";

const SHARE_DECIMALS = 6;
const TARGET_CHAIN_ID = baseSepolia.id;

/**
 * useDolBalance — single source of truth for a user's pBondSenior holdings.
 *
 * Returns:
 *   - balanceShares: bigint | null    (raw on-chain balance in pBS units)
 *   - balance: number                 (human-readable pBS)
 *   - usdcValue: number               (USDC value via pricePerShare)
 *   - isLoading: while any read is in flight
 *   - hasBalance: balance > 0
 *   - refetch: call after a successful deposit/redeem to force update
 *
 * Uses `pricePerShare()` on pBondSenior for conversion. pBondSenior
 * starts at 1:1 (1 pBS = 1 USDC) and grows as the vault accrues.
 */
export function useDolBalance() {
  const config = getPBondConfig();
  const { address: userAddress, isConnected } = useAccount();
  const senior = config.senior;

  // Raw balance
  const {
    data: balanceRaw,
    isLoading: balanceLoading,
    refetch: refetchBalance,
  } = useReadContract({
    address: senior.address,
    abi: senior.abi as Abi,
    functionName: "balanceOf",
    args: userAddress ? [userAddress] : undefined,
    chainId: TARGET_CHAIN_ID,
    query: { enabled: !!userAddress, refetchInterval: 15_000 },
  });

  // Price per share — pBS → USDC ratio (6 decimals)
  const {
    data: pricePerShareRaw,
    isLoading: priceLoading,
    refetch: refetchPrice,
  } = useReadContract({
    address: senior.address,
    abi: senior.abi as Abi,
    functionName: "pricePerShare",
    chainId: TARGET_CHAIN_ID,
    query: { enabled: !!senior.address, refetchInterval: 30_000 },
  });

  const balanceShares =
    typeof balanceRaw === "bigint" ? balanceRaw : null;

  const balance =
    balanceShares !== null
      ? Number(balanceShares) / 10 ** SHARE_DECIMALS
      : 0;

  const pricePerShare =
    typeof pricePerShareRaw === "bigint"
      ? Number(pricePerShareRaw) / 10 ** SHARE_DECIMALS
      : 1; // 1:1 fallback

  const usdcValue = balance * pricePerShare;

  const refetch = () => {
    refetchBalance();
    refetchPrice();
  };

  // Auto-refresh on any dol tx confirmation (deposit, withdraw, claim)
  // or when the tab becomes visible after a route change / blur. Kills
  // the "I withdrew but the homepage still shows my old balance" bug.
  useEffect(() => {
    const unsub = onDolStateShouldRefresh(() => {
      refetchBalance();
      refetchPrice();
    });
    return unsub;
  }, [refetchBalance, refetchPrice]);

  return {
    isConnected,
    balanceShares, // raw bigint for redeem() calls
    balance, // human-readable pBS count
    usdcValue, // USDC value
    pricePerShare,
    hasBalance: balance > 0,
    isLoading: balanceLoading || priceLoading,
    refetch,
  };
}
