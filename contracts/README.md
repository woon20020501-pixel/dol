# contracts

Solidity contracts for the Dol vault and the DOL receipt token.

## Purpose

On-chain custody and share accounting for the two products in this monorepo:

- **PacificaCarryVault** — ERC-4626 USDC vault with a signed-NAV oracle, 70/30 margin/treasury split, and a two-step (request + cooldown + claim) withdraw queue.
- **Dol** — ERC-20 redeploy of the earlier senior-tranche asset. Phase 1 uses this as a plain transferable share; junior is inactive (`juniorContract = 0x0`).

The contracts hold user USDC, mint share tokens, enforce a ±10% sanity guard on every off-chain NAV submission, and route withdrawals first from idle USDC then from the treasury lending market (Moonwell-compatible) when necessary.

## Layout

```
packages/contracts/
├── foundry.toml              solc 0.8.24, via_ir, fuzz=256, invariant depth=32
├── remappings.txt
├── src/
│   ├── PacificaCarryVault.sol    949 lines — main vault, ERC-4626 + signed NAV
│   ├── Dol.sol                   260 lines — Phase-1 share token
│   ├── pBondJunior.sol           186 lines — junior tranche (inactive in Phase 1)
│   ├── IPBondJunior.sol           13 lines — junior interface
│   ├── IMoonwellMarket.sol        30 lines — ERC-4626 subset for treasury leg
│   └── MockMoonwellMarket.sol    116 lines — test double for local runs
├── test/    11 test suites, 196 test cases (see Testing)
├── script/
│   ├── Deploy.s.sol                  CarryVault deploy + shared/contracts.json emit
│   ├── DeployDol.s.sol               Dol deploy
│   ├── write-contracts-json.js       ffi helper invoked from Deploy.s.sol
│   └── write-dol-json.js             ffi helper for DeployDol.s.sol
└── lib/                          git submodules: forge-std, openzeppelin-contracts
```

## Key contract: `PacificaCarryVault`

ERC-4626 (`asset = USDC`, 6 decimals) plus the following extensions:

| Surface | Description |
|---|---|
| `deposit(assets, receiver)` | Standard ERC-4626; splits the deposit 70% into idle USDC and 30% into the configured Moonwell market via `_depositToTreasury`. `mint`, `withdraw`, `redeem` are disabled. |
| `requestWithdraw(shares, receiver)` | Burns shares immediately, records a `PendingWithdraw` with a `cooldownEndsAt` timestamp. Emits `WithdrawRequested`. |
| `claimWithdraw(requestId)` | After cooldown elapses, transfers USDC from idle + treasury redemption. Emits `WithdrawClaimed`. |
| `instantWithdraw(shares, receiver)` | Idle-USDC-only path with a 5 bps fee (`INSTANT_WITHDRAW_FEE_BPS`, compile-time constant). Reverts `InsufficientBalance()` if idle < required. |
| `reportNAV(value, signature)` | OPERATOR_ROLE signs `(operator, nonce, chainId, contractAddr, value, timestamp)` via EIP-191. `totalAssetsStored` updates only if `|Δ| ≤ 10%` of prior value (`NavDeltaTooLarge`). |
| `totalAssets()` | Returns `idleUsdc + treasuryUnderlying + totalAssetsStored`. |

**Roles:** `OPERATOR_ROLE` (bot, signs NAV reports), `GUARDIAN_ROLE` (pause + key rotation). **No `DEFAULT_ADMIN_ROLE` is granted** — role topology is fixed at deploy time.

**Custom errors** (gas-efficient): `VaultPaused`, `ZeroAssets`, `NotRequestOwner`, `CooldownNotElapsed`, `AlreadyClaimed`, `WithdrawDisabled`, `InvalidNavSignature`, `StaleTimestamp`, `NavDeltaTooLarge`, `TreasuryMintFailed(uint256)`, `TreasuryRedeemFailed(uint256)`, `InsufficientBalance`.

## Dependencies

- **OpenZeppelin 5.x** — `ERC4626`, `ERC20`, `SafeERC20`, `ReentrancyGuard`, `AccessControl`, `ECDSA`, `MessageHashUtils`. Pinned via git submodule in `lib/openzeppelin-contracts`.
- **forge-std** — test harness, cheatcodes, fuzzing.
- **Moonwell compatibility** — the treasury leg depends on an `IMoonwellMarket` ERC-4626 adapter. Production points at a live Moonwell USDC market; local tests use `MockMoonwellMarket` which models mint/redeem error codes plus a simulated `exchangeRate` drift.

## Testing

| Suite | Cases | Coverage |
|---|---|---|
| `PacificaCarryVault.t.sol` | 56 | Deposit, request/claim withdraw, instant withdraw, NAV report signing, pause, role admin, share price after NAV, error paths |
| `ERC7540Compliance.t.sol` | 34 | Async-withdraw queue parity with EIP-7540 semantics: request → pending → claim, share burn timing, cooldown invariance under additional deposits |
| `Dol.t.sol` | 23 | Mint, burn, transfer, approve, junior-contract address immutability |
| `pBondJunior.t.sol` | 21 | Junior tranche lifecycle (inactive path included — mints & burns should revert in Phase 1) |
| `SecurityAttacks.t.sol` | 11 | Reentrancy, signature replay, stale-timestamp, cross-chain replay, pause bypass, role escalation |
| `MockMoonwellMarket.t.sol` | 11 | Mock fidelity: exchange-rate accrual, error-code plumbing, precision loss bounds |
| `PacificaCarryVault.navReport.t.sol` | 11 | NAV signing flow end-to-end (EIP-191 payload bytes, nonce increment, sanity guard both sides of ±10%, golden vectors for operator reproduction) |
| `PacificaCarryVault.invariant.t.sol` | 10 | `shares ↔ assets` monotonicity, `totalSupply ≤ totalAssets / minSharePrice`, request queue conservation over random call sequences (depth 32, 64 runs) |
| `PacificaCarryVault.fuzz.t.sol` | 9 | Fuzzed deposit/withdraw sequences; 256 runs each |
| `Scenarios.t.sol` | 6 | End-to-end: first deposit, NAV drift, normal claim, instant path, pause + unpause, operator rotation |
| `DifferentialOZ4626.t.sol` | 4 | Differential-oracle sanity: overloaded ERC-4626 accounting matches reference OZ implementation on the allowed surface |

Run: `forge test` (full) or `forge test --match-path test/PacificaCarryVault.*` (vault-only).

Current result: **196 passed, 0 failed, 0 skipped.**

### Invariant testing

`PacificaCarryVault.invariant.t.sol` uses forge's stateful invariant runner with `runs=64` and `depth=32` (from `foundry.toml`). Handler functions randomize deposits, NAV updates, requests, and claims; invariants assert accounting closure after every call sequence.

## Deployment

Deploy scripts use `ffi` to write `shared/contracts.json` so sibling packages (dashboard, bot) can read the deployed addresses without manual copying. This is enabled explicitly in `foundry.toml`:

```toml
ffi = true
```

```bash
# CarryVault (production)
forge script script/Deploy.s.sol \
  --rpc-url $BASE_SEPOLIA_RPC \
  --broadcast \
  --private-key $DEPLOYER_PK

# Dol token (after vault is live)
forge script script/DeployDol.s.sol --rpc-url $BASE_SEPOLIA_RPC --broadcast
```

## Integration points

- **Bot (the Rust runtime)** — holds the `OPERATOR_ROLE` key. Calls `reportNAV` on the 5-minute cycle; the signed payload format is a byte-exact port of `test/PacificaCarryVault.navReport.t.sol`.
- **Dashboard** — reads `shared/contracts.json` at build time to get addresses and ABI fragments; uses wagmi to render `totalAssets`, `sharePrice`, `balanceOf`, pending request queue, and NAV report history (from `NavReported` events).
- **Strategy (Python research layer)** — never touches chain. Consumes only the signal JSON produced by the bot.

## Ship status

| Component | Status |
|---|---|
| PacificaCarryVault | Deployable on Base Sepolia; not audited |
| Dol ERC-20 | Deployed on Base Sepolia (see `shared/contracts.json`) |
| pBondJunior | Deactivated in Phase 1 — junior mint/burn paths revert |
| NAV-signer key rotation | Implemented via `grantRole(OPERATOR_ROLE, new)` + `revokeRole(OPERATOR_ROLE, old)` |
| External audit | Not completed |
| Upgradeability | None — contracts are non-upgradeable by design. Redeploy required for changes |
