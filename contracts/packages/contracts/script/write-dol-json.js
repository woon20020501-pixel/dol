#!/usr/bin/env node
/**
 * write-dol-json.js
 *
 * Called by DeployDol.s.sol via FFI after deployment.
 * Updates shared/contracts.json with the new Dol address. Phase 1 uses the
 * legacy `pBondSenior` key for backward compatibility with the dashboard's
 * config-driven pipeline — the key stays, only the value changes. A `dol`
 * alias is also written for forward clarity. Atomic write (tmp + rename).
 *
 * Usage: node script/write-dol-json.js <vault> <usdc> <dol> <blockNumber>
 */
const fs = require("fs");
const path = require("path");

const [vault, usdc, dol, blockNumber] = process.argv.slice(2);

const repoRoot = path.join(__dirname, "..", "..", "..");
const finalPath = path.join(repoRoot, "shared", "contracts.json");
const tmpPath = finalPath + ".tmp";

let existing = {};
if (fs.existsSync(finalPath)) {
  existing = JSON.parse(fs.readFileSync(finalPath, "utf8"));
}

// Keep the legacy pBondSenior key pointing at the new Dol address so the
// dashboard's existing config reads keep working without code changes.
existing.pBondSenior = dol;
// Add a forward-looking alias.
existing.dol = dol;
existing.dolDeployedAt = Number(blockNumber);

fs.writeFileSync(tmpPath, JSON.stringify(existing, null, 2) + "\n", "utf8");
fs.renameSync(tmpPath, finalPath);

process.stdout.write("written Dol address to " + finalPath);
