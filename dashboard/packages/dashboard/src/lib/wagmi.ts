import { createConfig } from "@privy-io/wagmi";
import { http } from "wagmi";
import { baseSepolia } from "wagmi/chains";

/**
 * Wagmi config for the Pacifica Carry Vault dashboard.
 *
 * Uses Base Sepolia as the default chain.
 *
 * ── RPC host hardening ────────────────────────────────────────────
 *
 * `NEXT_PUBLIC_RPC_URL` is a build-time env var. If the Vercel
 * project's secrets are compromised and an attacker swaps it to
 * a malicious RPC, the dashboard's contract reads would start
 * believing a lying chain — balances could be faked, tx receipts
 * could lie about confirmation. We can't prevent the secrets
 * compromise itself (that's an access control problem), but we
 * CAN validate the configured URL against a known-safe host
 * allowlist at module init. A non-matching value falls back to
 * the public Base Sepolia endpoint and logs a loud warning.
 *
 * The allowlist is explicit — any time we add a new RPC provider
 * (Alchemy, Infura, QuickNode), it has to be added here first.
 * This is a second source of truth alongside the env var and
 * catches bulk swaps via a build-time mistake or attack.
 */

const SAFE_RPC_HOSTS: ReadonlySet<string> = new Set([
  "sepolia.base.org",
  "base-sepolia.blockpi.network",
  "base-sepolia-rpc.publicnode.com",
  "base-sepolia.gateway.tenderly.co",
]);

const DEFAULT_RPC = "https://sepolia.base.org";

function resolveRpcUrl(): string {
  const raw = process.env.NEXT_PUBLIC_RPC_URL;
  if (!raw) return DEFAULT_RPC;
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== "https:") {
      if (typeof window !== "undefined") {
        // eslint-disable-next-line no-console
        console.error(
          `[wagmi] NEXT_PUBLIC_RPC_URL must be https, got ${parsed.protocol}. Falling back.`,
        );
      }
      return DEFAULT_RPC;
    }
    // Allow Alchemy / Infura subdomains on their known bases.
    const host = parsed.hostname.toLowerCase();
    const isAlchemy = host.endsWith(".g.alchemy.com");
    const isInfura = host.endsWith(".infura.io");
    if (SAFE_RPC_HOSTS.has(host) || isAlchemy || isInfura) {
      return raw;
    }
    if (typeof window !== "undefined") {
      // eslint-disable-next-line no-console
      console.error(
        `[wagmi] NEXT_PUBLIC_RPC_URL host "${host}" is not on the safe allowlist. Falling back to ${DEFAULT_RPC}.`,
      );
    }
    return DEFAULT_RPC;
  } catch {
    return DEFAULT_RPC;
  }
}

const rpcUrl = resolveRpcUrl();

export const wagmiConfig = createConfig({
  chains: [baseSepolia],
  transports: {
    [baseSepolia.id]: http(rpcUrl),
  },
});

export { baseSepolia };
