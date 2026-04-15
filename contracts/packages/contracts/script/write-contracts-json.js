#!/usr/bin/env node
/**
 * write-contracts-json.js
 *
 * Called by Deploy.s.sol via FFI after a successful deployment.
 * Reads the vault ABI from the Foundry compiled artifact and writes
 * shared/contracts.json atomically (write to .tmp, then rename).
 *
 * Usage: node script/write-contracts-json.js <chainId> <vault> <usdc> <treasuryVault> <deployer> <operator> <guardian> <timestamp>
 */
const fs = require("fs");
const path = require("path");

const [chainId, vault, usdc, treasuryVault, deployer, operator, guardian, timestamp] =
  process.argv.slice(2);

// Read the full ABI from the Foundry compiled artifact
const artifactPath = path.join(
  __dirname,
  "..",
  "out",
  "PacificaCarryVault.sol",
  "PacificaCarryVault.json"
);
const artifact = JSON.parse(fs.readFileSync(artifactPath, "utf8"));

const output = {
  chainId: Number(chainId),
  vault,
  usdc,
  treasuryVault,
  allocation: {
    treasuryBps: 3000,
    marginBps: 7000,
  },
  abi: artifact.abi,
  deployedAt: Number(timestamp),
  deployer,
  operator,
  guardian,
};

// Atomic write: temp file then rename
const repoRoot = path.join(__dirname, "..", "..", "..");
const finalPath = path.join(repoRoot, "shared", "contracts.json");
const tmpPath = finalPath + ".tmp";

// Ensure shared/ directory exists
const sharedDir = path.dirname(finalPath);
if (!fs.existsSync(sharedDir)) {
  fs.mkdirSync(sharedDir, { recursive: true });
}

fs.writeFileSync(tmpPath, JSON.stringify(output, null, 2) + "\n", "utf8");
fs.renameSync(tmpPath, finalPath);

process.stdout.write("written to " + finalPath);
