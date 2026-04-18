/**
 * pBond tranche contract configuration.
 *
 * Reads pBondSenior/pBondJunior addresses from shared/contracts.json
 * when available; falls back to the addresses from the PM's deploy message.
 * USDC address is read from the vault config.
 *
 * ── Runtime integrity checks ──────────────────────────────────────
 *
 * Even though `shared/contracts.json` is trusted repo input, we
 * treat the values as untrusted at runtime and apply three sanity
 * checks on every `getPBondConfig()` call:
 *
 *   1. chainId MUST be 84532 (Base Sepolia). If a compromised build
 *      ships with a mainnet chainId pointing at an attacker contract,
 *      the check refuses to return a config and forces the fallback.
 *   2. Every address MUST match `0x` + 40 hex. Malformed or empty
 *      strings trigger fallback.
 *   3. The MockUSDC address is compared against a known hardcoded
 *      value. Anything else falls back.
 *
 * Compared to the fallback-only check we had before, this closes
 * the window where a repo attacker can swap addresses in
 * contracts.json and have them silently propagate to production.
 */

import type { Address } from "viem";
import { getVaultConfig } from "./vault";
import pBondSeniorAbiJson from "@/abi/pBondSenior.json";
import pBondJuniorAbiJson from "@/abi/pBondJunior.json";
import { isValidAddress } from "./guards";

// Dol contract fallback addresses (Base Sepolia, 2026-04-14)
// Primary source is shared/contracts.json — these only trigger if JSON
// is missing or the fields are absent. Updated to match the Dol rename
// redeploy (commit 7c54f94). pBondSenior key is retained in JSON for
// pipeline compatibility, but the value is the new Dol contract address.
// pBondJunior is inactive in Phase 1 (juniorContract set to 0x0 on the
// new Dol); kept as a conservative default.
const PBOND_SENIOR_FALLBACK: Address = "0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4";
const PBOND_JUNIOR_FALLBACK: Address = "0x08858aDA7F681204BB89a9fA80a3179D3Df567fB";
const USDC_FALLBACK: Address = "0xEEC3C8bA0d09d86ccbb23f982875C00B716009bD";
const BASE_SEPOLIA_CHAIN_ID = 84532;

// Known-good addresses lock. Any value read from contracts.json that
// doesn't match the below is rejected and the fallback is used. This
// means a repo attacker can't swap the address to drain user funds —
// they'd also have to modify this file, which is a second, obvious
// change that catches code review. When the contracts are redeployed,
// update both the fallback constant AND this allowlist in the same commit.
const KNOWN_SENIOR_ADDRESSES: ReadonlySet<string> = new Set([
  PBOND_SENIOR_FALLBACK.toLowerCase(),
]);
const KNOWN_JUNIOR_ADDRESSES: ReadonlySet<string> = new Set([
  PBOND_JUNIOR_FALLBACK.toLowerCase(),
]);
const KNOWN_USDC_ADDRESSES: ReadonlySet<string> = new Set([
  USDC_FALLBACK.toLowerCase(),
]);

export const pBondSeniorAbi = pBondSeniorAbiJson as readonly Record<string, unknown>[];
export const pBondJuniorAbi = pBondJuniorAbiJson as readonly Record<string, unknown>[];

export type TrancheType = "senior" | "junior";

function safeAddress(
  value: unknown,
  fallback: Address,
  allowlist: ReadonlySet<string>,
  label: string,
): Address {
  if (!isValidAddress(value)) {
    if (process.env.NODE_ENV !== "production") {
      // eslint-disable-next-line no-console
      console.warn(
        `[pbond] ${label} in contracts.json is not a valid address; falling back to ${fallback}`,
      );
    }
    return fallback;
  }
  const lower = value.toLowerCase();
  if (!allowlist.has(lower)) {
    // eslint-disable-next-line no-console
    console.error(
      `[pbond] ${label} address ${value} is not in the known allowlist. ` +
        `Refusing to use it. Falling back to ${fallback}. If this is a ` +
        `legitimate redeploy, update KNOWN_${label.toUpperCase()}_ADDRESSES ` +
        `in pbond.ts.`,
    );
    return fallback;
  }
  return value;
}

export function getPBondConfig() {
  const vault = getVaultConfig();

  // chainId sanity: only Base Sepolia is supported in Phase 1. A
  // config pointing at mainnet or another testnet is either a
  // misdeploy or an attack and we refuse to honor it.
  const rawChainId = vault?.chainId ?? BASE_SEPOLIA_CHAIN_ID;
  const chainId =
    rawChainId === BASE_SEPOLIA_CHAIN_ID ? rawChainId : BASE_SEPOLIA_CHAIN_ID;
  if (rawChainId !== BASE_SEPOLIA_CHAIN_ID) {
    // eslint-disable-next-line no-console
    console.error(
      `[pbond] chainId ${rawChainId} is not Base Sepolia (${BASE_SEPOLIA_CHAIN_ID}). ` +
        `Forcing ${BASE_SEPOLIA_CHAIN_ID} and using fallback addresses.`,
    );
  }

  const usdcAddress = safeAddress(
    vault?.usdcAddress,
    USDC_FALLBACK,
    KNOWN_USDC_ADDRESSES,
    "usdc",
  );

  const seniorAddress = safeAddress(
    vault?.pBondSenior,
    PBOND_SENIOR_FALLBACK,
    KNOWN_SENIOR_ADDRESSES,
    "pBondSenior",
  );

  const juniorAddress = safeAddress(
    vault?.pBondJunior,
    PBOND_JUNIOR_FALLBACK,
    KNOWN_JUNIOR_ADDRESSES,
    "pBondJunior",
  );

  return {
    senior: {
      address: seniorAddress,
      abi: pBondSeniorAbi,
      symbol: "pBond-S",
      label: "Senior",
    },
    junior: {
      address: juniorAddress,
      abi: pBondJuniorAbi,
      symbol: "pBond-J",
      label: "Junior",
    },
    usdcAddress,
    chainId,
  };
}
