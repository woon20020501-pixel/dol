# On-chain & verified

Dol is fully on-chain on Base Sepolia testnet. Every contract is public, every transaction is verifiable, and every line of code is published on BaseScan. You do not have to take our word for any of it.

## The contracts

These are the live addresses on Base Sepolia (chain ID 84532) as of the Phase 1 launch:

| Contract | Address | What it does |
|---|---|---|
| **Dol token** | `0x9E6Cc40CC68Ef1bf46Fcab5574E10771B7566Db4` | The Dol ERC-20 token you hold in your wallet. Mintable on deposit, burnable on cash-out. |
| **Pacifica Carry Vault** | `0x5F1330A074b074aD8f461191aCf48229fB364ca5` | The vault that holds pooled USDC and runs the strategy. ERC-4626 standard. |
| **Treasury Vault** | `0x2448E2ABDD9647c9741502E6863b97F8583A0074` | Where the on-chain treasury portion sits, earning low-risk base returns. |
| **Test USDC** | `0xEEC3C8bA0d09d86ccbb23f982875C00B716009bD` | The Base Sepolia test USDC used by Phase 1. Has no real-world value. |

All four are publicly readable. Click any address above and you will land on BaseScan, where you can read the verified source code, browse every transaction, and call any read function yourself.

## Source code

Every contract is **verified on BaseScan**. That means the bytecode running at the address above matches the human-readable Solidity source we publish. You can read it under each contract's `Contract → Code` tab.

The contracts are non-upgradeable in the standard sense — there is no proxy admin that can swap the implementation overnight. Any future change requires a fresh deployment, a new address, and a public announcement.

## Tests

Before deployment, the contract suite passes:

- **117 unit and fuzz tests** covering deposit, redeem, instant redeem, scheduled redeem, fee math, liquidity checks, and pause states
- **100 percent line coverage** on the Pacifica Carry Vault (the contract that holds the pool)
- **97.65 percent line coverage** on the Dol token (the wrapper you interact with)
- **Slither static analysis: zero high-severity, zero medium-severity findings**

The full test report and coverage data live in the `contracts` directory. The tests are open source and you are welcome to run them yourself.

## How to verify yourself

If you want to confirm what we say is true, here are three quick checks anyone can run.

**Check 1: the Dol token is what we say it is.**

Open BaseScan at the Dol token address. Under `Read Contract`, find the `name()` and `symbol()` functions and call them. They should return `"Dol"` and `"DOL"`. Then call `decimals()` — it should return `6`, matching USDC.

**Check 2: the vault is holding USDC.**

Open BaseScan at the Pacifica Carry Vault address. Under `Read Contract`, call `asset()` — it should return the test USDC address listed above. Then call `totalAssets()` — this is the live pool size in test USDC. Compare against `totalSupply()` to compute the price-per-share at this exact moment.

**Check 3: the price-per-share is what we say it is.**

On the same Dol token contract, call `pricePerShare()`. This is the live USDC value of one Dol, computed on-chain from `totalAssets() / totalSupply()`. Whatever the dashboard displays for the per-Dol value should match this number to the last decimal — if it doesn't, trust the chain, not the website.

## What is *not* yet verified

- **No third-party audit yet.** The 117 tests + Slither are internal verification. A formal audit by a security firm is planned before any mainnet launch. Phase 1 is testnet-only and uses test USDC with no real value, so the audit gap is acceptable for this phase but not for mainnet.
- **No formal verification of the Pacifica DEX integration.** We rely on Pacifica's own audits and uptime. If Pacifica has a problem, Dol has a problem. See the [Risk Disclosure](/legal/risk) Section 4 for the strategy-side risks.
- **No on-chain governance yet.** The Dol team makes all parameter and upgrade decisions. There is no token vote.

For the full list of things we have *not* verified and the things you must trust the team on, read the [Risk Disclosure](/legal/risk).
