"use client";

import { useCallback, useEffect, useState } from "react";
import {
  useAccount,
  useReadContract,
  useWriteContract,
  useWaitForTransactionReceipt,
  useSwitchChain,
  useChainId,
} from "wagmi";
import { parseUnits, formatUnits, maxUint256 } from "viem";
import { getVaultConfig, ERC20_ABI, VAULT_ABI } from "@/lib/vault";
import { emitDolTxConfirmed } from "@/lib/txEvents";
import { TARGET_CHAIN_ID } from "@/lib/chains";
import { translateError } from "@/lib/errors";

const USDC_DECIMALS = 6;
const EXPECTED_CHAIN_ID = TARGET_CHAIN_ID;

export function useDeposit() {
  const vaultConfig = getVaultConfig();
  const { address: userAddress, isConnected } = useAccount();
  const chainId = useChainId();
  const { switchChain } = useSwitchChain();

  const isDemoMode = process.env.NEXT_PUBLIC_DEMO_MODE === "true";
  const isWrongNetwork = isConnected && chainId !== EXPECTED_CHAIN_ID;

  // ── Reads ───────────────────────────────────────────────────────

  // USDC address from contracts.json (no RPC call needed)
  const usdcAddress = vaultConfig?.usdcAddress;

  // USDC balance
  const { data: usdcBalanceRaw, refetch: refetchBalance } = useReadContract({
    address: usdcAddress,
    abi: ERC20_ABI,
    functionName: "balanceOf",
    args: [userAddress!],
    query: {
      enabled: !!usdcAddress && !!userAddress,
      refetchInterval: 10_000,
    },
  });

  // USDC allowance for vault
  const { data: allowanceRaw, refetch: refetchAllowance } = useReadContract({
    address: usdcAddress,
    abi: ERC20_ABI,
    functionName: "allowance",
    args: [userAddress!, vaultConfig?.address ?? ("0x0" as `0x${string}`)],
    query: {
      enabled: !!usdcAddress && !!userAddress && !!vaultConfig,
      refetchInterval: 5_000,
    },
  });

  const usdcBalance =
    usdcBalanceRaw !== undefined
      ? Number(formatUnits(usdcBalanceRaw as bigint, USDC_DECIMALS))
      : null;

  const allowance = (allowanceRaw as bigint) ?? BigInt(0);

  // ── Approve write ───────────────────────────────────────────────

  const {
    writeContract: doApprove,
    data: approveHash,
    error: approveError,
    isPending: isApprovePending,
    reset: resetApprove,
  } = useWriteContract();

  const {
    isLoading: isApproveConfirming,
    isSuccess: isApproveConfirmed,
  } = useWaitForTransactionReceipt({ hash: approveHash });

  // Refetch allowance after approval confirms. Track the refetch itself so
  // the UI can hold off on enabling the Deposit button until the on-chain
  // allowance read has actually updated — otherwise there's a window where
  // isApproveConfirmed is true but `allowance` still reads the old value,
  // causing the button to flicker back to "Approve USDC" or (worse) to
  // submit a deposit tx that fails because of a transient race.
  const [isRefetchingAllowance, setIsRefetchingAllowance] = useState(false);

  useEffect(() => {
    if (!isApproveConfirmed) return;
    let cancelled = false;
    setIsRefetchingAllowance(true);
    refetchAllowance().finally(() => {
      if (!cancelled) setIsRefetchingAllowance(false);
    });
    return () => {
      cancelled = true;
    };
  }, [isApproveConfirmed, refetchAllowance]);

  // ── Deposit write ───────────────────────────────────────────────

  const {
    writeContract: doDeposit,
    data: depositHash,
    error: depositError,
    isPending: isDepositPending,
    reset: resetDeposit,
  } = useWriteContract();

  const {
    isLoading: isDepositConfirming,
    isSuccess: isDepositConfirmed,
  } = useWaitForTransactionReceipt({ hash: depositHash });

  // Refetch local balance + broadcast a global tx-confirmed event so
  // every downstream read hook (useDolBalance on the homepage,
  // useVaultReads on /dashboard, LiveVaultTicker on the landing,
  // SystemHealthSection, etc.) can refetch its own state in the same
  // frame. No more stale "my Dol is gone" UI after a tx.
  useEffect(() => {
    if (isDepositConfirmed) {
      refetchBalance();
      emitDolTxConfirmed("deposit", depositHash);
    }
  }, [isDepositConfirmed, refetchBalance, depositHash]);

  // Approve confirmations also broadcast so allowance-watching UIs
  // (elsewhere in the app) refresh in sync.
  useEffect(() => {
    if (isApproveConfirmed) {
      emitDolTxConfirmed("approve", approveHash);
    }
  }, [isApproveConfirmed, approveHash]);

  // ── Actions ─────────────────────────────────────────────────────

  const needsApproval = useCallback(
    (amount: number): boolean => {
      if (!amount || amount <= 0) return false;
      const amountWei = parseUnits(amount.toString(), USDC_DECIMALS);
      return allowance < amountWei;
    },
    [allowance],
  );

  // Approve max uint256 instead of the exact deposit amount. Three wins:
  //   1. User approves once, then every subsequent deposit skips the
  //      2-step flow and goes straight to the deposit tx.
  //   2. No JS Number → parseUnits precision edge cases where allowance
  //      ends up 1 wei short of the deposit amount.
  //   3. The post-approval refetch race becomes a non-issue — even stale
  //      allowance reads show >> deposit amount.
  const approve = useCallback(() => {
    if (!vaultConfig || !usdcAddress) return;
    // Double-click guard: refuse to send a second approve while the
    // first is still pending or confirming. Stops the UI from racing
    // two parallel approval txs if the user taps fast.
    if (isApprovePending || isApproveConfirming) return;
    doApprove({
      address: usdcAddress,
      abi: ERC20_ABI,
      functionName: "approve",
      args: [vaultConfig.address, maxUint256],
    });
  }, [vaultConfig, usdcAddress, doApprove, isApprovePending, isApproveConfirming]);

  const deposit = useCallback(
    (amount: number) => {
      if (!vaultConfig || !userAddress) return;
      // Double-click guard: refuse rapid re-submission while a deposit
      // is already in flight OR while the post-approval allowance read
      // is still catching up (the latter is enforced by the UI too but
      // defense in depth — UI could be bypassed via devtools).
      if (isDepositPending || isDepositConfirming || isRefetchingAllowance) return;
      // Defensive pre-parse: if amount is NaN / negative / zero / non
      // finite, drop silently rather than letting parseUnits throw.
      if (!Number.isFinite(amount) || amount <= 0) return;
      let amountWei: bigint;
      try {
        // Clamp display precision to USDC's 6 decimals so floats like
        // 1.23456789 can't make parseUnits throw "fractional too long".
        amountWei = parseUnits(amount.toFixed(USDC_DECIMALS), USDC_DECIMALS);
      } catch {
        return;
      }
      if (amountWei <= BigInt(0)) return;
      doDeposit({
        address: vaultConfig.address,
        abi: VAULT_ABI,
        functionName: "deposit",
        args: [amountWei, userAddress],
      });
    },
    [
      vaultConfig,
      userAddress,
      doDeposit,
      isDepositPending,
      isDepositConfirming,
      isRefetchingAllowance,
    ],
  );

  const reset = useCallback(() => {
    resetApprove();
    resetDeposit();
  }, [resetApprove, resetDeposit]);

  const switchNetwork = useCallback(() => {
    switchChain({ chainId: EXPECTED_CHAIN_ID });
  }, [switchChain]);

  // ── Derived state ───────────────────────────────────────────────

  const isApproving = isApprovePending || isApproveConfirming;
  const isDepositing = isDepositPending || isDepositConfirming;

  // Route both approve and deposit errors through translateError so
  // the UI never leaks raw wagmi strings like
  // "ContractFunctionRevertedError: ... executionReverted (0x)".
  const approveErrorMessage = approveError
    ? translateError(approveError).description ?? translateError(approveError).title
    : null;
  const depositErrorMessage = depositError
    ? translateError(depositError).description ?? translateError(depositError).title
    : null;

  return {
    // State
    deployed: !!vaultConfig,
    isDemoMode,
    isWrongNetwork,
    isConnected,
    // Reads
    usdcBalance,
    // Approve
    isApproving,
    isApproveConfirmed,
    isRefetchingAllowance,
    approveError: approveErrorMessage,
    needsApproval,
    approve,
    // Deposit
    isDepositing,
    isDepositConfirmed,
    depositHash,
    approveHash,
    depositError: depositErrorMessage,
    deposit,
    // Util
    reset,
    switchNetwork,
  };
}
