# Security — PacificaCarryVault

> **Status**: Hackathon build. Not audited. No bug bounty.

---

## Roles

| Role | Holder | Capabilities |
|------|--------|-------------|
| `OPERATOR_ROLE` | Bot key | Sign and submit NAV reports via `reportNAV` |
| `GUARDIAN_ROLE` | governance key | `pause`, `unpause`, `setOperator`, `setGuardian` |
| `DEFAULT_ADMIN_ROLE` | **Nobody** | Not granted at deployment. No role escalation possible. |

**Separation of duties**: The operator cannot pause the vault or rotate keys. The guardian cannot submit NAV reports. Both roles are single-key (no multisig — acknowledged as a limitation).

**Rotation procedure**:
- Operator: Guardian calls `setOperator(newAddress)`. Old operator loses `OPERATOR_ROLE` immediately; old signatures become invalid.
- Guardian: Guardian calls `setGuardian(newAddress)`. Old guardian loses `GUARDIAN_ROLE` immediately and cannot reverse the change.

---

## Trust Assumptions

1. **Operator is honest**: The operator key signs NAV reports that determine the share price. A compromised operator can drift NAV by up to 9.99% per report. The 10% sanity guard limits single-report damage but cannot prevent gradual drift over many reports.
2. **Guardian is honest**: The guardian can pause the vault (blocking deposits and claims) and rotate both keys. A compromised guardian could lock the vault permanently or transfer control.
3. **Off-chain NAV oracle is accurate**: The vault's share price reflects the operator-reported NAV, not the actual USDC balance. If the off-chain strategy loses more than the NAV reports indicate, the vault is undercollateralized.
4. **USDC is a well-behaved ERC-20**: No fee-on-transfer, no rebasing, standard `transferFrom` behavior.

---

## Threat Model

### In-scope threats (mitigated)

| Threat | Mitigation |
|--------|-----------|
| **Unauthorized NAV report** | ECDSA signature verification against `operator` address |
| **Signature replay** | Timestamp monotonicity (`timestamp > lastTimestamp`) |
| **Catastrophic NAV manipulation** | 10% sanity guard per report (`delta * 10 >= lastNav` reverts) |
| **Reentrancy on deposit** | `nonReentrant` modifier (OpenZeppelin ReentrancyGuard) |
| **Reentrancy on claimWithdraw** | `nonReentrant` modifier + checks-effects-interactions pattern |
| **Reentrancy on reportNAV** | `nonReentrant` modifier |
| **Integer overflow/underflow** | Solidity 0.8.24 built-in overflow checks |
| **First-depositor inflation attack** | OpenZeppelin ERC4626 virtual shares (offset of 1) |
| **Unauthorized pause/unpause** | `onlyRole(GUARDIAN_ROLE)` modifier |
| **Unauthorized key rotation** | `onlyRole(GUARDIAN_ROLE)` modifier |
| **Double-claim of withdrawal** | `req.claimed` flag checked before payout |
| **Early withdrawal claim** | `block.timestamp >= unlockTimestamp` enforced |

### Out-of-scope threats (not mitigated)

| Threat | Why out of scope |
|--------|-----------------|
| **Guardian collusion/compromise** | Single-key model; multisig is future work |
| **Operator compromise beyond 10% drift** | Gradual drift over many reports is possible; monitoring is off-chain |
| **USDC depeg or blacklisting** | External dependency; vault does not hedge USDC risk |
| **Block timestamp manipulation** | Miners can drift ±15s; cooldown is 24h, so impact is negligible |
| **Front-running deposits/withdrawals** | MEV protection is out of scope for hackathon |

---

## Invariants

Proven by Foundry invariant tests (64 runs × 32 depth each):

1. **`invariant_totalAssetsNeverNegative`**: `totalAssets()` is always ≥ 0. The uint256 slot cannot underflow (Solidity 0.8 reverts on underflow).

2. **`invariant_sharePriceMonotonicExceptOnLoss`**: Share price only decreases when `reportNAV` is called with a lower NAV (loss) or when `claimWithdraw` returns the price from its post-requestWithdraw inflated level. Deposits alone never decrease the share price beyond ERC4626 integer rounding dust (< 10 ppb).

3. **`invariant_pausedBlocksDeposits`**: While paused, `deposit()` always reverts with `VaultPaused()` regardless of caller, asset amount, or receiver.

4. **`invariant_onlyOperatorReportsNav`**: No non-operator address can successfully call `reportNAV`. Fuzzed across random callers and random private keys.

---

## Emergency Procedures

### When to pause
- Suspected operator key compromise
- Off-chain strategy experiencing abnormal losses
- USDC instability or depeg event
- Any situation requiring immediate halt of fund flows

### How to pause
```
cast send <vault> "pause()" --private-key <guardian_pk> --rpc-url <rpc>
```

### How to rotate operator (compromised bot key)
```
cast send <vault> "setOperator(address)" <new_operator> --private-key <guardian_pk> --rpc-url <rpc>
```

### How to rotate guardian (compromised governance key)
```
cast send <vault> "setGuardian(address)" <new_guardian> --private-key <guardian_pk> --rpc-url <rpc>
```

> **Warning**: If the guardian key is lost or compromised and set to address(0), the vault is permanently locked in its current pause state with no way to rotate keys. This is a known limitation of the single-key model.

---

## Known Limitations

1. **ERC-4626 partial compliance**: `withdraw()`, `redeem()`, and `mint()` are disabled (always revert). Users must use the two-step `requestWithdraw` + `claimWithdraw` queue. This means the vault does not fully comply with the ERC-4626 standard.

2. **NAV sanity guard at 10%**: The guard prevents single-report jumps of ≥ 10%, but cannot detect gradual drift. An attacker with the operator key could drift NAV by ~9.99% per report over multiple reports.

3. **Single-key access model**: Both operator and guardian are single EOA keys. No multisig, no timelock, no social recovery. Key loss is permanent.

4. **No DEFAULT_ADMIN_ROLE**: No address has `DEFAULT_ADMIN_ROLE`, which means roles cannot be granted or revoked via the standard AccessControl `grantRole`/`revokeRole` — only via the guardian's `setOperator`/`setGuardian` functions.

5. **Withdraw queue price risk**: Between `requestWithdraw` and `claimWithdraw`, the locked asset amount is fixed at the share price at request time. If NAV changes during the cooldown, the claimant gets the original amount regardless.

6. **No on-chain NAV validation**: The contract trusts the operator's signed NAV within the 10% bound. It has no way to independently verify the off-chain portfolio value.

---

## Slither Findings

Scan: `slither . --filter-paths "lib/|test/|script/"` (slither-analyzer 0.11.5)

**HIGH**: 0
**MEDIUM**: 0

### LOW findings (Vault)

| # | Detector | Location | Resolution |
|---|----------|----------|-----------|
| 1 | `missing-zero-check` | `constructor._operator` | **Accepted**: Deployer is trusted. Zero-address operator would simply make `reportNAV` unusable (no valid signatures). Guardian can rotate via `setOperator`. |
| 2 | `missing-zero-check` | `constructor._guardian` | **Accepted**: Deployer is trusted. Zero-address guardian would lock pause/rotation permanently — documented in Known Limitations. |
| 3 | `missing-zero-check` | `setOperator.newOperator` | **Accepted by design**: Setting operator to address(0) is a valid emergency action to disable NAV reporting. Documented in NatSpec `@custom:security`. |
| 4 | `missing-zero-check` | `setGuardian.newGuardian` | **Accepted risk**: Setting guardian to address(0) is irreversible and dangerous, but the guardian is trusted. Documented in NatSpec `@custom:security` and Known Limitations. |
| 5 | `timestamp` | `claimWithdraw` | **Accepted**: `block.timestamp` comparison against a 24-hour cooldown. Miner manipulation of ±15 seconds is negligible relative to 86400 seconds. |

### LOW findings (pBond wrappers)

| # | Detector | Location | Resolution |
|---|----------|----------|-----------|
| 6 | `missing-zero-check` | `pBondSenior.setJuniorContract` | **Accepted**: One-time call by guardian during deployment. Zero address would make distributeYield unusable. |
| 7 | `missing-zero-check` | `pBondJunior.setSeniorContract` | **Accepted**: One-time call during deployment. |
| 8 | `reentrancy-benign` | `pBondSenior.redeem` | **Accepted**: State written after `vault.requestWithdraw()` is the redeem request record. Protected by `nonReentrant`. Vault is trusted immutable. |
| 9 | `reentrancy-benign` | `pBondJunior.redeem` | **Accepted**: Same as #8. |
| 10 | `reentrancy-events` | `pBondSenior.distributeYield` | **Accepted**: Event emitted after `absorbLoss()` external call. Junior is a trusted immutable contract. No state corruption risk. |
| 11 | `timestamp` | `pBondSenior.distributeYield` | **Accepted**: Timestamp used for yield accrual calculation. Miner manipulation is negligible for APY computation over days/weeks. |
| 12 | `naming-convention` | `pBondSenior`, `pBondJunior` | **Accepted**: Deliberate branding choice — "pBond" is the product name. |

### INFORMATIONAL findings

None after fixes (immutable-states resolved).

---

## Audit Status

- **Formal audit**: None. This is a hackathon prototype.
- **Bug bounty**: N/A.
- **Static analysis**: Slither 0.11.5 — 0 high, 0 medium, 12 low (all accepted with justification).
- **Test coverage**: 100% (vault), 97% (pBondSenior), 93% (pBondJunior).
- **Total tests**: 103 (75 vault + 28 pBond).
- **Invariant testing**: 4 invariants, 64 runs × 32 depth each.
- **Fuzz testing**: 9 fuzz tests, 256 runs each.
