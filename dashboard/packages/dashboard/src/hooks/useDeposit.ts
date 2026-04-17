"use client";

import { useCallback, useEffect, useState } from "react";
import {
  useAccount,
  useBalance,
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
import {
  pickRevertedHash,
  anyReverted,
  mergeTxError,
  isRevertError,
} from "@/lib/txState";

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

  // Native ETH balance — used for gas pre-flight. If it's below the
  // LOW_GAS threshold, the UI warns before the user signs, so they
  // don't burn time on a tx that will revert with "insufficient funds
  // for gas". Much better UX than MetaMask's post-popup gas error.
  const { data: ethBalance } = useBalance({
    address: userAddress,
    chainId: EXPECTED_CHAIN_ID,
    query: {
      enabled: !!userAddress,
      refetchInterval: 20_000,
    },
  });

  // Rough estimate: approve + deposit ~ 200k gas @ 0.001 gwei on
  // Base Sepolia ≈ 0.0000002 ETH. Using 0.0005 ETH as the floor so
  // even a 50× gas spike still clears. Chosen over a true
  // estimateGas call because (a) we want to warn BEFORE the user
  // fills in an amount, and (b) estimateGas requires a signed
  // unsigned tx scaffold which is the awkward part of wagmi.
  const LOW_GAS_ETH = 0.0005;
  const ethBalanceNum = ethBalance
    ? Number(formatUnits(ethBalance.value, ethBalance.decimals))
    : null;
  const hasLowGas = ethBalanceNum !== null && ethBalanceNum < LOW_GAS_ETH;

  // Zero-address sentinel for wagmi's useReadContract args. wagmi
  // requires a concrete args tuple even when `enabled: false` gates
  // the actual call, because the tuple is read during hook setup.
  // We pass ZERO_ADDR when userAddress is missing; the `enabled` flag
  // ensures the RPC call never fires with it.
  const ZERO_ADDR = "0x0000000000000000000000000000000000000000" as const;
  const readerAddress = userAddress ?? ZERO_ADDR;

  // USDC balance
  const { data: usdcBalanceRaw, refetch: refetchBalance } = useReadContract({
    address: usdcAddress,
    abi: ERC20_ABI,
    functionName: "balanceOf",
    args: [readerAddress],
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
    args: [readerAddress, vaultConfig?.address ?? ZERO_ADDR],
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

  // Receipt-query error fires when `waitForTransactionReceipt` throws.
  // wagmi core 2.22.1 throws for TWO different causes:
  //   (a) receipt.status === 'reverted' → plain `new Error(reason)`
  //   (b) RPC transport failure → a named viem error subclass
  // `hasApproveReceiptError` is true in BOTH cases; we derive the
  // narrower `isApproveReverted` via isRevertError() so the UI only
  // shows "Reverted on-chain" for actual reverts, not RPC hiccups.
  const {
    isLoading: isApproveConfirming,
    isSuccess: isApproveConfirmed,
    isError: hasApproveReceiptError,
    error: approveReceiptError,
  } = useWaitForTransactionReceipt({ hash: approveHash });
  const isApproveReverted =
    hasApproveReceiptError && isRevertError(approveReceiptError);

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
    isError: hasDepositReceiptError,
    error: depositReceiptError,
  } = useWaitForTransactionReceipt({ hash: depositHash });
  const isDepositReverted =
    hasDepositReceiptError && isRevertError(depositReceiptError);

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

  // Merge wallet-rejection errors with receipt-query errors (reverted
  // tx) — writeContract errors take precedence. Pipe through
  // translateError so the UI never leaks raw wagmi internals.
  const approveCombined = mergeTxError(approveError, approveReceiptError);
  const depositCombined = mergeTxError(depositError, depositReceiptError);
  const approveErrorMessage = approveCombined
    ? translateError(approveCombined).description ??
      translateError(approveCombined).title
    : null;
  const depositErrorMessage = depositCombined
    ? translateError(depositCombined).description ??
      translateError(depositCombined).title
    : null;

  // Revert-specific fields so the UI can render "Reverted on-chain"
  // and deep-link to basescan. Distinct from wallet-rejection errors
  // where there is no tx hash to link to. See txState.test.ts for
  // the contract of pickRevertedHash / anyReverted.
  const revertFlows = [
    { isReverted: isApproveReverted, hash: approveHash },
    { isReverted: isDepositReverted, hash: depositHash },
  ];
  const revertedHash = pickRevertedHash(revertFlows);
  const isReverted = anyReverted(revertFlows);

  return {
    // State
    deployed: !!vaultConfig,
    isDemoMode,
    isWrongNetwork,
    isConnected,
    // Reads
    usdcBalance,
    ethBalance: ethBalanceNum,
    hasLowGas,
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
    // Reverted-tx state: consumers can render a "View on Basescan"
    // link instead of a flat error toast when isReverted is true.
    isReverted,
    revertedHash,
    deposit,
    // Util
    reset,
    switchNetwork,
  };
}
