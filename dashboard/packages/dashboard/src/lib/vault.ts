/**
 * Reads vault contract config from shared/contracts.json.
 *
 * Decision: we read at import time (build time for server components,
 * module load time for client). This is fine because the contract
 * address doesn't change at runtime — it's set once at deploy.
 * If the file doesn't exist or address is null, we return null and
 * the dashboard falls back to demo mode.
 */

import type { Abi, Address } from "viem";
import { log } from "@/lib/logger";

export type VaultConfig = {
  address: Address;
  abi: Abi;
  usdcAddress: Address;
  chainId: number;
  deployedAt: number;
  treasuryVault: Address | null;
  allocation: { treasuryBps: number; marginBps: number };
  pBondSenior: Address | null;
  pBondJunior: Address | null;
};

const DEFAULT_ALLOCATION = { treasuryBps: 3000, marginBps: 7000 };

let _vaultConfig: VaultConfig | null = null;
let _loaded = false;

/**
 * Parses shared/contracts.json (flat schema from the deploy script):
 *   { chainId, vault, usdc, abi, deployedAt, deployer, operator, guardian }
 */
function loadVaultConfig(): VaultConfig | null {
  if (_loaded) return _vaultConfig;
  _loaded = true;

  let contracts: Record<string, unknown>;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    contracts = require("../../../../shared/contracts.json");
  } catch {
    log.warn("[vault] shared/contracts.json not found — falling back to demo mode.");
    return null;
  }

  if (!contracts.vault || typeof contracts.vault !== "string") {
    log.warn("[vault] shared/contracts.json has no vault address — falling back to demo mode.");
    return null;
  }

  if (!contracts.abi || !Array.isArray(contracts.abi)) {
    log.warn("[vault] shared/contracts.json has no abi — falling back to demo mode.");
    return null;
  }

  const allocationRaw = contracts.allocation as
    | { treasuryBps?: number; marginBps?: number }
    | undefined;
  const allocation =
    allocationRaw &&
    typeof allocationRaw.treasuryBps === "number" &&
    typeof allocationRaw.marginBps === "number"
      ? { treasuryBps: allocationRaw.treasuryBps, marginBps: allocationRaw.marginBps }
      : DEFAULT_ALLOCATION;

  const treasuryVault =
    typeof contracts.treasuryVault === "string" && contracts.treasuryVault.startsWith("0x")
      ? (contracts.treasuryVault as Address)
      : null;

  const pBondSenior =
    typeof contracts.pBondSenior === "string" && contracts.pBondSenior.startsWith("0x")
      ? (contracts.pBondSenior as Address)
      : null;
  const pBondJunior =
    typeof contracts.pBondJunior === "string" && contracts.pBondJunior.startsWith("0x")
      ? (contracts.pBondJunior as Address)
      : null;

  _vaultConfig = {
    address: contracts.vault as Address,
    abi: contracts.abi as Abi,
    usdcAddress: (contracts.usdc as Address) ?? ("0x0" as Address),
    chainId: (contracts.chainId as number) ?? 84532,
    deployedAt: (contracts.deployedAt as number) ?? 0,
    treasuryVault,
    allocation,
    pBondSenior,
    pBondJunior,
  };
  return _vaultConfig;
}

export function getVaultConfig(): VaultConfig | null {
  return loadVaultConfig();
}

// ── ABI constants (from INTERFACES.md section 1) ────────────────────

/**
 * MockMoonwellMarket / Moonwell-style market ABI subset.
 * `balanceOfUnderlying(account)` returns the USDC value of the account's
 * mTokens at the current exchange rate. Used to read the vault's treasury
 * position without doing the rate math on the client.
 */
export const MOONWELL_ABI = [
  {
    name: "balanceOfUnderlying",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "account", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "supplyRatePerSecond",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "uint256" }],
  },
] as const;

/** Minimal ERC-20 ABI for USDC reads + approval */
export const ERC20_ABI = [
  {
    name: "balanceOf",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "account", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "allowance",
    type: "function",
    stateMutability: "view",
    inputs: [
      { name: "owner", type: "address" },
      { name: "spender", type: "address" },
    ],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "approve",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [
      { name: "spender", type: "address" },
      { name: "amount", type: "uint256" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
] as const;

/**
 * Vault ABI subset for dashboard read + write operations.
 * Matches IPacificaCarryVault in INTERFACES.md section 1.
 */
export const VAULT_ABI = [
  // ERC-4626 user-facing
  {
    name: "deposit",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [
      { name: "assets", type: "uint256" },
      { name: "receiver", type: "address" },
    ],
    outputs: [{ name: "shares", type: "uint256" }],
  },
  {
    name: "requestWithdraw",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [{ name: "shares", type: "uint256" }],
    outputs: [{ name: "requestId", type: "uint256" }],
  },
  {
    name: "claimWithdraw",
    type: "function",
    stateMutability: "nonpayable",
    inputs: [{ name: "requestId", type: "uint256" }],
    outputs: [{ name: "assets", type: "uint256" }],
  },
  // Views
  {
    name: "asset",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "address" }],
  },
  {
    name: "totalAssets",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "sharePrice",
    type: "function",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    name: "balanceOf",
    type: "function",
    stateMutability: "view",
    inputs: [{ name: "account", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  // Events
  {
    name: "Deposit",
    type: "event",
    inputs: [
      { name: "user", type: "address", indexed: true },
      { name: "assets", type: "uint256", indexed: false },
      { name: "shares", type: "uint256", indexed: false },
    ],
  },
  {
    name: "WithdrawRequested",
    type: "event",
    inputs: [
      { name: "id", type: "uint256", indexed: true },
      { name: "user", type: "address", indexed: false },
      { name: "shares", type: "uint256", indexed: false },
    ],
  },
  {
    name: "WithdrawClaimed",
    type: "event",
    inputs: [
      { name: "id", type: "uint256", indexed: true },
      { name: "user", type: "address", indexed: false },
      { name: "assets", type: "uint256", indexed: false },
    ],
  },
] as const;
