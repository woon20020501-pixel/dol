"use client";

import { usePrivy } from "@privy-io/react-auth";
import { LogOut } from "lucide-react";

export function ConnectButton() {
  const { ready, authenticated, login, logout, user } = usePrivy();

  if (!ready) {
    return (
      <div className="h-9 w-32 animate-pulse rounded-full bg-dark-surface-2" />
    );
  }

  if (authenticated && user) {
    const displayLabel = getDisplayLabel(user);

    return (
      <div className="flex items-center gap-3">
        <span className="hidden text-[12px] text-dark-secondary sm:block font-mono">
          {displayLabel}
        </span>
        <button
          type="button"
          onClick={logout}
          className="flex items-center gap-1.5 rounded-full border border-dark-border bg-dark-surface px-4 py-2 text-[12px] font-medium text-dark-primary transition-colors hover:border-dark-border-strong hover:bg-dark-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-senior focus-visible:ring-offset-2 focus-visible:ring-offset-dark-bg"
          aria-label="Disconnect wallet"
        >
          <LogOut className="h-3.5 w-3.5" />
          Disconnect
        </button>
      </div>
    );
  }

  return (
    <button
      type="button"
      onClick={login}
      className="rounded-full bg-senior px-5 py-2 text-[13px] font-medium text-dark-bg transition-colors hover:bg-senior-dark focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-senior focus-visible:ring-offset-2 focus-visible:ring-offset-dark-bg"
    >
      Connect Wallet
    </button>
  );
}

function getDisplayLabel(user: NonNullable<ReturnType<typeof usePrivy>["user"]>): string {
  if (user.email?.address) {
    return user.email.address;
  }

  if (user.google?.email) {
    return user.google.email;
  }

  const wallet = user.wallet;
  if (wallet?.address) {
    return truncateAddress(wallet.address);
  }

  return "Connected";
}

function truncateAddress(address: string): string {
  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}
