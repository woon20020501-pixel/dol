# PacificaCarryVault — Contracts

ERC-4626 style vault for the Pacifica FX Carry strategy. Accepts USDC deposits, issues shares, and uses a signed NAV oracle to track off-chain positions.

## Build

```bash
forge build
```

## Test

```bash
# All tests (75 vault + 28 pBond = 103 total)
forge test

# Verbose output
forge test -vv

# Coverage
forge coverage --report summary
```

### Coverage (Phase 4)

| File | Lines | Statements | Branches | Functions |
|------|-------|------------|----------|-----------|
| PacificaCarryVault.sol | 100.00% (77/77) | 100.00% (79/79) | 100.00% (14/14) | 100.00% (15/15) |

### Slither (Phase 6)

```bash
slither . --filter-paths "lib/|test/|script/"
```

| Severity | Count |
|----------|-------|
| High | 0 |
| Medium | 0 |
| Low | 17 (all accepted — see [SECURITY.md](SECURITY.md)) |

## Deploy

### Prerequisites

- [Foundry](https://book.getfoundry.sh/getting-started/installation)
- Node.js (for contracts.json writer)
- Base Sepolia ETH in deployer wallet

### Configuration

Copy `.env.example` to `.env` and fill in:

```bash
DEPLOYER_PRIVATE_KEY=0x...
OPERATOR_ADDRESS=0x...
GUARDIAN_ADDRESS=0x...
BASE_SEPOLIA_RPC_URL=https://sepolia.base.org
# Optional: USDC_ADDRESS (deploys mock if unset)
```

### Deploy to Base Sepolia

```bash
source .env
forge script script/Deploy.s.sol --tc Deploy \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --broadcast --ffi
```

This will:
1. Deploy MockUSDC (if `USDC_ADDRESS` is not set)
2. Deploy PacificaCarryVault
3. Write `shared/contracts.json` with vault address, ABI, and metadata

### Verify on Basescan

```bash
forge verify-contract <VAULT_ADDRESS> src/PacificaCarryVault.sol:PacificaCarryVault \
  --chain base-sepolia \
  --constructor-args $(cast abi-encode "constructor(address,address,address,uint256)" <USDC> <OPERATOR> <GUARDIAN> 86400) \
  --etherscan-api-key $ETHERSCAN_API_KEY
```

## Role Management

| Role | Holder | Can do |
|------|--------|--------|
| `OPERATOR_ROLE` | Bot | `reportNAV` (signed NAV updates) |
| `GUARDIAN_ROLE` | governance | `pause`, `unpause`, `setOperator`, `setGuardian` |

### Rotate operator

```bash
cast send <vault> "setOperator(address)" <new_operator> \
  --private-key <guardian_pk> --rpc-url <rpc>
```

### Rotate guardian

```bash
cast send <vault> "setGuardian(address)" <new_guardian> \
  --private-key <guardian_pk> --rpc-url <rpc>
```

## pBond Tranche Structure

The vault's yield is split into two ERC-20 tranches:

### pBond-S (Senior)
- Address: `0xd90E69b17A0c030b5984F9376C675BE37D257397`
- Target APY: 7.5%
- Priority yield, last-to-lose
- Use case: treasury, conservative LPs

### pBond-J (Junior)
- Address: `0xa86E7e5EB609c9fBAffa7a78E5bCd1c25ff9344D`
- Residual yield (whatever Senior leaves)
- First-loss buffer (absorbs losses before Senior)
- Use case: quants, risk-tolerant LPs

### Deposit / Redeem Flow
1. User calls `pBond.deposit(usdcAmount)`
2. pBond contract approves + calls `vault.deposit()`
3. User receives pBS or pBJ tokens 1:1 with USDC
4. Yield accrues automatically via `vault.distributeYield()`
5. User calls `pBond.redeem(tokenAmount)` to request withdrawal
6. 24h cooldown (same as vault's requestWithdraw pattern)
7. User calls `pBond.claimRedeem()` to receive USDC

### Deploy pBond Wrappers

```bash
source .env
VAULT_ADDRESS=0xD08C1C78E3Fc6Ac007C06F2b73a28eA8b057A522 \
USDC_ADDRESS=0xEEC3C8bA0d09d86ccbb23f982875C00B716009bD \
forge script script/DeployPBondWrappers.s.sol \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --broadcast --ffi
```

## Security

See [SECURITY.md](SECURITY.md) for the full threat model, invariants, emergency procedures, known limitations, and Slither findings.

## Current Deployment

| Network | Contract | Address |
|---------|----------|---------|
| Base Sepolia | Vault | `0xD08C1C78E3Fc6Ac007C06F2b73a28eA8b057A522` |
| Base Sepolia | USDC (mock) | `0xEEC3C8bA0d09d86ccbb23f982875C00B716009bD` |
| Base Sepolia | Treasury | `0x2448E2ABDD9647c9741502E6863b97F8583A0074` |
| Base Sepolia | pBondSenior | `0xd90E69b17A0c030b5984F9376C675BE37D257397` |
| Base Sepolia | pBondJunior | `0xa86E7e5EB609c9fBAffa7a78E5bCd1c25ff9344D` |
