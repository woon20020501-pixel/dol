// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {Dol} from "../src/Dol.sol";
import {pBondJunior} from "../src/pBondJunior.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title DolTest
/// @notice Tests for the pBond Senior tranche wrapper: deposit, redeem,
///         yield distribution, price mechanics, and integration with Junior.
contract DolTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;
    Dol senior;
    pBondJunior junior;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    uint256 constant COOLDOWN = 86400; // 24 hours

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);

        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));
        usdc.mint(address(treasury), 10_000_000e6);

        vault = new PacificaCarryVault(
            IERC20(address(usdc)),
            treasury,
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        senior = new Dol(vault, IERC20(address(usdc)), guardian);
        junior = new pBondJunior(vault, IERC20(address(usdc)));

        // Link tranches
        vm.prank(guardian);
        senior.setJuniorContract(address(junior));
        junior.setSeniorContract(address(senior));

        // Fund test users
        usdc.mint(alice, 100_000e6);
        usdc.mint(bob, 100_000e6);
        vm.prank(alice);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(alice);
        usdc.approve(address(junior), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(junior), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 1. DEPOSIT — MINTS 1:1
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Depositing USDC mints DOL tokens 1:1.
    /// Protects: the 1:1 minting invariant for Senior tranche.
    function test_deposit_mints1to1() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        assertEq(senior.balanceOf(alice), 1000e6, "DOL minted 1:1");
        assertEq(senior.totalDeposited(), 1000e6, "totalDeposited tracked");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 2. DEPOSIT — APPROVES AND CALLS VAULT
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Deposit routes USDC through the vault (vault receives shares).
    /// Protects: Senior holds vault shares proportional to deposit.
    function test_deposit_approvesAndCallsVault() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        uint256 seniorVaultShares = IERC20(address(vault)).balanceOf(address(senior));
        assertTrue(seniorVaultShares > 0, "Senior holds vault shares");
        // Vault shares should be convertible back to ~1000 USDC
        uint256 value = vault.convertToAssets(seniorVaultShares);
        assertEq(value, 1000e6, "Vault shares worth deposited amount");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 3. REDEEM — BURNS AND RETURNS USDC (two-step)
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Full redeem flow: deposit -> redeem -> warp -> claimRedeem.
    /// Protects: the two-step withdraw queue round-trips correctly.
    function test_redeem_burnsAndReturnsUSDC() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        vm.prank(alice);
        uint256 redeemId = senior.redeem(1000e6);

        assertEq(senior.balanceOf(alice), 0, "DOL burned");

        // Warp past cooldown
        vm.warp(block.timestamp + COOLDOWN);

        uint256 balBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        senior.claimRedeem(redeemId);

        uint256 received = usdc.balanceOf(alice) - balBefore;
        assertEq(received, 1000e6, "USDC returned in full");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 4. REDEEM — REFLECTS CURRENT NAV
    // ═══════════════════════════════════════════════════════════════════

    /// @notice After a NAV increase, pricePerShare and vault value reflect
    ///         the appreciation. Partial redeem returns proportionally more.
    /// Protects: vault share appreciation is reflected in redemption value.
    function test_redeem_reflectsCurrentNAV() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        // Increase vault NAV via reportNAV (first report skips guard)
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(200e6, ts, _signNav(200e6, ts));

        // Senior's vault shares are now worth more than deposited
        uint256 seniorShares = IERC20(address(vault)).balanceOf(address(senior));
        uint256 seniorValue = vault.convertToAssets(seniorShares);
        assertTrue(seniorValue > 1000e6, "Senior value increased");

        // pricePerShare should reflect the gain
        uint256 price = senior.pricePerShare();
        assertTrue(price > 1e6, "Price per share reflects NAV increase");

        // Partial redeem (within idle+treasury liquidity): redeem 500 DOL
        vm.prank(alice);
        uint256 redeemId = senior.redeem(500e6);
        vm.warp(block.timestamp + COOLDOWN);

        uint256 balBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        senior.claimRedeem(redeemId);
        uint256 received = usdc.balanceOf(alice) - balBefore;
        assertTrue(received > 500e6, "Received more than deposited portion after NAV increase");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 5. DISTRIBUTE YIELD — SENIOR GETS TARGET
    // ═══════════════════════════════════════════════════════════════════

    /// @notice After yield distribution, Senior retains up to its 7.5% APY target.
    /// Protects: the yield cap mechanism for Senior tranche.
    function test_distributeYield_senior_getsTarget() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // Report NAV to generate yield
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(500e6, ts, _signNav(500e6, ts));

        // Warp 365 days for full year yield calc
        vm.warp(block.timestamp + 365 days);

        uint256 seniorValueBefore = vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(senior))
        );

        senior.distributeYield();

        uint256 seniorValueAfter = vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(senior))
        );

        // Senior should retain principal + 7.5% yield = 1075e6
        // Allow small rounding tolerance
        assertApproxEqAbs(seniorValueAfter, 1075e6, 2e6, "Senior retains target yield");
        assertTrue(seniorValueBefore > seniorValueAfter, "Senior gave excess to Junior");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 6. DISTRIBUTE YIELD — SENIOR CAPS AT TARGET
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Senior does not keep more than its target APY — excess goes to Junior.
    /// Protects: Senior yield is capped, not open-ended.
    function test_distributeYield_senior_capsAtTarget() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // Large NAV increase
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(1000e6, ts, _signNav(1000e6, ts));

        vm.warp(block.timestamp + 365 days);

        uint256 juniorSharesBefore = IERC20(address(vault)).balanceOf(address(junior));
        senior.distributeYield();
        uint256 juniorSharesAfter = IERC20(address(vault)).balanceOf(address(junior));

        assertTrue(juniorSharesAfter > juniorSharesBefore, "Junior received excess shares");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7. DISTRIBUTE YIELD — SENIOR GETS LESS WHEN VAULT YIELD LOW
    // ═══════════════════════════════════════════════════════════════════

    /// @notice When vault yield is below Senior's target, Senior keeps all
    ///         available yield and Junior gets nothing.
    /// Protects: low-yield scenario — Senior takes what's available.
    function test_distributeYield_senior_getsLessWhenVaultYieldLow() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // No NAV increase — vault yield is only from treasury accrual
        vm.warp(block.timestamp + 30 days);

        uint256 juniorSharesBefore = IERC20(address(vault)).balanceOf(address(junior));
        senior.distributeYield();
        uint256 juniorSharesAfter = IERC20(address(vault)).balanceOf(address(junior));

        // With only treasury yield (~5% APY on 30% of assets), Senior's
        // proportional gain is small and likely below 7.5% APY target.
        // No excess should flow to Junior.
        assertEq(juniorSharesAfter, juniorSharesBefore, "Junior gets no excess when yield is low");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 8. SET JUNIOR CONTRACT — ONLY ONCE
    // ═══════════════════════════════════════════════════════════════════

    /// @notice setJuniorContract can only be called once by guardian.
    /// Protects: immutability of the Senior-Junior link after setup.
    function test_setJuniorContract_onlyOnce() public {
        // Already set in setUp(), so trying again should revert
        vm.prank(guardian);
        vm.expectRevert(Dol.JuniorAlreadySet.selector);
        senior.setJuniorContract(address(junior));
    }

    // ═══════════════════════════════════════════════════════════════════
    // 9. PRICE PER SHARE — INCREASES OVER TIME
    // ═══════════════════════════════════════════════════════════════════

    /// @notice pricePerShare reflects vault appreciation before distribution.
    /// Protects: price accuracy for Senior holders.
    function test_pricePerShare_increasesOverTime() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        uint256 priceBefore = senior.pricePerShare();
        assertEq(priceBefore, 1e6, "Initial price is 1:1");

        // Report NAV to increase vault value
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(200e6, ts, _signNav(200e6, ts));

        uint256 priceAfter = senior.pricePerShare();
        assertTrue(priceAfter > priceBefore, "Price increased after NAV report");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 10. REDEEM — REVERTS IF INSUFFICIENT BALANCE
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Redeeming more DOL than owned reverts.
    /// Protects: balance check before burn.
    function test_redeem_revertsIfInsufficientBalance() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        vm.prank(alice);
        vm.expectRevert(Dol.InsufficientBalance.selector);
        senior.redeem(2000e6);
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTRA: ZERO AMOUNT REVERTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Depositing zero reverts.
    /// Protects: zero-amount guard.
    function test_deposit_zeroAmount_reverts() public {
        vm.prank(alice);
        vm.expectRevert(Dol.ZeroAmount.selector);
        senior.deposit(0);
    }

    /// @notice Redeeming zero reverts.
    /// Protects: zero-amount guard on redeem.
    function test_redeem_zeroAmount_reverts() public {
        vm.prank(alice);
        vm.expectRevert(Dol.ZeroAmount.selector);
        senior.redeem(0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTRA: GUARDIAN ONLY ON SET JUNIOR
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Non-guardian cannot set Junior contract.
    /// Protects: access control on setJuniorContract.
    function test_setJuniorContract_nonGuardian_reverts() public {
        // Deploy a fresh Senior to test (setUp already linked the one above)
        Dol freshSenior = new Dol(vault, IERC20(address(usdc)), guardian);

        vm.prank(alice);
        vm.expectRevert(Dol.OnlyGuardian.selector);
        freshSenior.setJuniorContract(address(junior));
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTRA: CLAIM REDEEM GUARDS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Only the request owner can claim.
    /// Protects: ownership check on claimRedeem.
    function test_claimRedeem_notOwner_reverts() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(alice);
        uint256 redeemId = senior.redeem(500e6);
        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(bob);
        vm.expectRevert(Dol.NotRedeemOwner.selector);
        senior.claimRedeem(redeemId);
    }

    /// @notice Double claim reverts.
    /// Protects: single-use claim tickets.
    function test_claimRedeem_doubleClaim_reverts() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(alice);
        uint256 redeemId = senior.redeem(1000e6);
        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(alice);
        senior.claimRedeem(redeemId);

        vm.prank(alice);
        vm.expectRevert(Dol.AlreadyClaimed.selector);
        senior.claimRedeem(redeemId);
    }

    // ═══════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════

    function _signNav(uint256 newNav, uint256 timestamp) internal view returns (bytes memory) {
        bytes32 payloadHash = keccak256(
            abi.encodePacked(
                "PACIFICA_CARRY_VAULT_NAV",
                address(vault),
                newNav,
                timestamp
            )
        );
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", payloadHash)
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(OPERATOR_PK, ethSignedHash);
        return abi.encodePacked(r, s, v);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 10. INSTANT REDEEM — Dol fast path
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Dol.instantRedeem passthrough burns DOL, calls
    ///         vault.instantRedeem, and forwards net USDC to the user.
    ///         Fee (5 bps) is routed to the vault-level feeRecipient,
    ///         not the Senior wrapper.
    /// Protects: the Dol Instant button — single-tx fast path without
    ///           cooldown. If this is broken, the Dol UX promise fails.
    function test_instantRedeem_senior_passthrough() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        uint256 aliceUsdcBefore = usdc.balanceOf(alice);
        uint256 guardianUsdcBefore = usdc.balanceOf(guardian);

        // Instant-redeem half her DOL (500e6)
        vm.prank(alice);
        uint256 out = senior.instantRedeem(500e6);

        uint256 expectedFee = (500e6 * 5) / 10_000;
        uint256 expectedNet = 500e6 - expectedFee;

        assertEq(out, expectedNet, "wrapper returns net");
        assertEq(usdc.balanceOf(alice) - aliceUsdcBefore, expectedNet, "alice got net USDC");
        assertEq(
            usdc.balanceOf(guardian) - guardianUsdcBefore,
            expectedFee,
            "fee routed to vault feeRecipient (guardian)"
        );
        assertEq(senior.balanceOf(alice), 500e6, "DOL half burned");
        assertEq(senior.totalDeposited(), 500e6, "principal halved");
    }
}
