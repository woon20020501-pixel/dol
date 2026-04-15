"use client";

import { useState, useEffect, useCallback } from "react";
import { useAccount } from "wagmi";
import { parseLocalStorageArray, isValidLoggedTx } from "@/lib/guards";

/**
 * Local-only transaction history keyed by wallet address.
 * Persists the last 50 user-initiated txs to localStorage so the
 * /my-dol recent activity list survives page refresh.
 *
 * This is PURE UX convenience — the chain is still the source of
 * truth. History entries are added when a tx is fired from this
 * session, never fetched from the chain.
 */

export type TxType =
  | "deposit"
  | "redeem-scheduled"
  | "redeem-instant"
  | "claim"
  | "approve";

export type TxStatus = "pending" | "confirmed" | "failed";

export interface LoggedTx {
  hash: `0x${string}`;
  type: TxType;
  amount: number; // USDC equivalent (human-readable)
  timestamp: number; // unix ms
  status: TxStatus;
}

const STORAGE_KEY = "dol_tx_history";
const MAX_ENTRIES = 50;

function storageKey(address: string): string {
  return `${STORAGE_KEY}_${address.toLowerCase()}`;
}

function loadHistory(address: string): LoggedTx[] {
  try {
    const raw = localStorage.getItem(storageKey(address));
    // Validated parse — malformed or injected entries get dropped
    return parseLocalStorageArray(raw, isValidLoggedTx) as LoggedTx[];
  } catch {
    return [];
  }
}

function saveHistory(address: string, history: LoggedTx[]) {
  try {
    localStorage.setItem(storageKey(address), JSON.stringify(history));
  } catch {
    // storage unavailable
  }
}

export function useTxHistory() {
  const { address } = useAccount();
  const [history, setHistory] = useState<LoggedTx[]>([]);

  // Load on mount / address change
  useEffect(() => {
    if (address) {
      setHistory(loadHistory(address));
    } else {
      setHistory([]);
    }
  }, [address]);

  /** Add a new pending tx. Returns the tx for chaining. */
  const record = useCallback(
    (params: {
      hash: `0x${string}`;
      type: TxType;
      amount: number;
    }): LoggedTx | null => {
      if (!address) return null;
      const tx: LoggedTx = {
        hash: params.hash,
        type: params.type,
        amount: params.amount,
        timestamp: Date.now(),
        status: "pending",
      };
      setHistory((prev) => {
        // Dedup by hash (a single tx can be recorded only once)
        const filtered = prev.filter((h) => h.hash !== tx.hash);
        const next = [tx, ...filtered].slice(0, MAX_ENTRIES);
        saveHistory(address, next);
        return next;
      });
      return tx;
    },
    [address],
  );

  /** Update status for a tx that's now confirmed or failed. */
  const updateStatus = useCallback(
    (hash: `0x${string}`, status: TxStatus) => {
      if (!address) return;
      setHistory((prev) => {
        const next = prev.map((h) =>
          h.hash === hash ? { ...h, status } : h,
        );
        saveHistory(address, next);
        return next;
      });
    },
    [address],
  );

  /** Clear all history for the current address. */
  const clear = useCallback(() => {
    if (!address) return;
    saveHistory(address, []);
    setHistory([]);
  }, [address]);

  return {
    history,
    record,
    updateStatus,
    clear,
    hasHistory: history.length > 0,
  };
}
