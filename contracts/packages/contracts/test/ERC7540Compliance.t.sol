// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title ERC7540ComplianceTest
/// @notice Proves the vault implements the EIP-7540 "Asynchronous ERC-4626
///         Tokenized Vault" async-redeem spec. Each test pins a specific
///         MUST/SHOULD requirement from the EIP text.
///
/// @dev References:
///      - EIP-7540 canonical text: https://eips.ethereum.org/EIPS/eip-7540
///      - Centrifuge reference impl: github.com/centrifuge/liquidity-pools
///        (`src/ERC7540Vault.sol`) — used to verify interface IDs and event
///        shapes against a production 7540 implementation.
///
/// Interface IDs (verified via Centrifuge reference, 2025-10):
///   IERC7540Redeem   = 0x620ee8e4
///   IERC7540Operator = 0xe3bc4e65
contract ERC7540ComplianceTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address charlie = makeAddr("charlie");
    uint256 constant COOLDOWN = 86400;

    bytes4 constant IERC7540_REDEEM_ID = 0x620ee8e4;
    bytes4 constant IERC7540_OPERATOR_ID = 0xe3bc4e65;
    bytes4 constant IERC165_ID = 0x01ffc9a7;

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);
        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));
        usdc.mint(address(treasury), 10_000_000e6);
        vault = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian, 0, 0
        );

        usdc.mint(alice, 1_000_000e6);
        usdc.mint(bob, 1_000_000e6);
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(vault), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540.1 — ERC-165 interface declaration
    // ═══════════════════════════════════════════════════════════════════
    //
    // EIP-7540: "Smart contracts implementing this standard MUST implement
    // ERC-165 interface detection. The async redeem vault MUST return TRUE
    // for the interface ID 0x620ee8e4. Async vaults supporting operators
    // MUST return TRUE for 0xe3bc4e65."

    function test_7540_supportsInterface_redeem() public view {
        assertTrue(vault.supportsInterface(IERC7540_REDEEM_ID), "7540.1a");
    }

    function test_7540_supportsInterface_operator() public view {
        assertTrue(vault.supportsInterface(IERC7540_OPERATOR_ID), "7540.1b");
    }

    function test_7540_supportsInterface_erc165() public view {
        assertTrue(vault.supportsInterface(IERC165_ID), "7540.1c");
    }

    function test_7540_supportsInterface_unknown() public view {
        assertFalse(vault.supportsInterface(0xdeadbeef), "7540.1d: unknown returns false");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540.2 — requestRedeem semantics
    // ═══════════════════════════════════════════════════════════════════
    //
    // EIP-7540: "requestRedeem MUST burn `shares` from `owner`. MUST emit
    // RedeemRequest event. MUST return a requestId."

    function test_7540_requestRedeem_burnsSharesFromOwner() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 aliceSharesBefore = vault.balanceOf(alice);
        assertEq(aliceSharesBefore, 1000e6, "alice shares pre-request");

        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(500e6, alice, alice);

        assertEq(vault.balanceOf(alice), 500e6, "7540.2a: shares burned");
        assertEq(reqId, 0, "7540.2b: first request id is 0 (monotonic)");
    }

    function test_7540_requestRedeem_emitsRedeemRequestEvent() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.expectEmit(true, true, true, true);
        emit PacificaCarryVault.RedeemRequest(
            alice, // controller indexed
            alice, // owner indexed
            0, // requestId indexed
            alice, // sender
            300e6 // shares
        );
        vm.prank(alice);
        vault.requestRedeem(300e6, alice, alice);
    }

    /// @notice `requestRedeem` with ERC-20 allowance (pre-7540 compat path).
    function test_7540_requestRedeem_withAllowance() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.approve(bob, 500e6); // ERC-20 allowance, not 7540 operator

        vm.prank(bob);
        uint256 reqId = vault.requestRedeem(500e6, alice, alice);

        assertEq(vault.balanceOf(alice), 500e6, "7540.2c: alice's shares burned via allowance");
        (address controller,,,,) = vault.withdrawRequests(reqId);
        assertEq(controller, alice, "7540.2d: controller == alice");
        // Allowance consumed
        assertEq(vault.allowance(alice, bob), 0, "7540.2e: allowance consumed");
    }

    /// @notice `requestRedeem` with EIP-7540 `setOperator` authorization.
    /// @dev Spec-mandatory path. Fix A (2026-04-17): prior to this fix,
    ///      `requestRedeem` consulted only ERC-20 allowance and ignored
    ///      `setOperator` approvals, violating EIP-7540 §"Methods".
    function test_7540_requestRedeem_withSetOperator() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        // Alice authorizes Bob as an EIP-7540 operator (NOT ERC-20 allowance)
        vm.prank(alice);
        vault.setOperator(bob, true);
        assertEq(vault.allowance(alice, bob), 0, "pre: no allowance");

        // Bob requests redeem on Alice's behalf — must succeed via operator
        vm.prank(bob);
        uint256 reqId = vault.requestRedeem(500e6, alice, alice);
        assertEq(vault.balanceOf(alice), 500e6, "7540.2f: shares burned via operator");
        (address controller,,,,) = vault.withdrawRequests(reqId);
        assertEq(controller, alice, "7540.2g: controller == alice");
        // Operator relationship does NOT consume allowance
        assertEq(vault.allowance(alice, bob), 0, "7540.2h: allowance still zero");
    }

    /// @notice `requestRedeem` without authorization reverts.
    function test_7540_requestRedeem_noAuth_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        // No allowance, no operator

        vm.prank(bob);
        vm.expectRevert();
        vault.requestRedeem(500e6, alice, alice);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540.3 — pendingRedeemRequest / claimableRedeemRequest views
    // ═══════════════════════════════════════════════════════════════════
    //
    // EIP-7540: "pendingRedeemRequest MUST return the pending share amount
    // that a controller has in the queue. claimableRedeemRequest MUST
    // return the claimable (cooldown-elapsed) share amount."

    function test_7540_pendingRedeemRequest_reflectsPending() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);

        assertEq(
            vault.pendingRedeemRequest(reqId, alice), 400e6, "7540.3a: pending shares"
        );
        assertEq(
            vault.claimableRedeemRequest(reqId, alice),
            0,
            "7540.3b: not claimable pre-cooldown"
        );
    }

    function test_7540_claimable_afterCooldown() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);

        vm.warp(block.timestamp + COOLDOWN + 1);

        assertEq(vault.pendingRedeemRequest(reqId, alice), 0, "7540.3c: no longer pending");
        assertEq(
            vault.claimableRedeemRequest(reqId, alice),
            400e6,
            "7540.3d: now claimable"
        );
    }

    function test_7540_views_zeroForOther() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);

        assertEq(
            vault.pendingRedeemRequest(reqId, bob), 0, "7540.3e: 0 for other controller"
        );
        assertEq(
            vault.claimableRedeemRequest(reqId, bob),
            0,
            "7540.3f: 0 for other controller"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540.4 — redeem(shares, receiver, controller) claim overload
    // ═══════════════════════════════════════════════════════════════════
    //
    // EIP-7540: "redeem(shares, receiver, controller) MUST consume from
    // claimableRedeemRequest. Caller MUST be controller or approved
    // operator. assets are determined at claim."

    function test_7540_redeem_claim_paysReceiver() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        uint256 bobBefore = usdc.balanceOf(bob);
        vm.prank(alice);
        uint256 assets = vault.redeem(400e6, bob, alice); // pay bob on alice's behalf
        uint256 bobDelta = usdc.balanceOf(bob) - bobBefore;

        assertGt(assets, 0, "7540.4a: assets returned");
        assertEq(bobDelta, assets, "7540.4b: receiver got assets");
    }

    function test_7540_redeem_claim_rejectsNonControllerNonOperator() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        // Bob is neither controller nor operator
        vm.prank(bob);
        vm.expectRevert(PacificaCarryVault.NotControllerOrOperator.selector);
        vault.redeem(400e6, bob, alice);
    }

    function test_7540_redeem_claim_acceptsApprovedOperator() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        // Alice approves bob as 7540 operator
        vm.prank(alice);
        vault.setOperator(bob, true);

        vm.prank(bob);
        uint256 assets = vault.redeem(400e6, charlie, alice);
        assertGt(assets, 0, "7540.4c: operator can claim on behalf");
        assertEq(usdc.balanceOf(charlie), assets, "7540.4d: receiver is charlie");
    }

    function test_7540_redeem_claim_shareMismatch_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AsyncClaimShareMismatch.selector);
        vault.redeem(399e6, alice, alice); // close but not exact
    }

    function test_7540_redeem_claim_preCooldown_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        // No warp — request is not yet claimable

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AsyncClaimNoReadyRequest.selector);
        vault.redeem(400e6, alice, alice);
    }

    /// @notice The assets-keyed async claim via `withdraw(assets, receiver,
    ///         controller)` matches a ready request by its locked assets.
    /// Protects: EIP-7540 ERC-4626-compat `withdraw` entrypoint.
    function test_7540_withdraw_claim_paysReceiver() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        // Look up locked assets for the request
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        uint256 bobBefore = usdc.balanceOf(bob);
        vm.prank(alice);
        uint256 shares = vault.withdraw(lockedAssets, bob, alice);
        uint256 bobDelta = usdc.balanceOf(bob) - bobBefore;

        assertEq(shares, 400e6, "7540.4e: withdraw returns shares burned");
        assertEq(bobDelta, lockedAssets, "7540.4f: receiver got assets");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540.5 — setOperator / isOperator
    // ═══════════════════════════════════════════════════════════════════

    function test_7540_setOperator_setAndCheck() public {
        assertFalse(vault.isOperator(alice, bob), "7540.5a: default false");

        vm.prank(alice);
        bool ok = vault.setOperator(bob, true);
        assertTrue(ok, "7540.5b: setOperator returns true");
        assertTrue(vault.isOperator(alice, bob), "7540.5c: state updated");

        vm.prank(alice);
        vault.setOperator(bob, false);
        assertFalse(vault.isOperator(alice, bob), "7540.5d: can revoke");
    }

    function test_7540_setOperator_emitsEvent() public {
        vm.expectEmit(true, true, false, true);
        emit PacificaCarryVault.OperatorSet(alice, bob, true);
        vm.prank(alice);
        vault.setOperator(bob, true);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 7540 coverage-gap tests — edge paths of new code
    // ═══════════════════════════════════════════════════════════════════

    /// @notice withdraw async-claim rejects non-controller non-operator.
    function test_7540_withdraw_claim_rejectsNonAuthorized() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        vm.prank(bob);
        vm.expectRevert(PacificaCarryVault.NotControllerOrOperator.selector);
        vault.withdraw(lockedAssets, bob, alice);
    }

    /// @notice withdraw async-claim: zero assets reverts ZeroAssets.
    function test_7540_withdraw_claim_zeroAssets_reverts() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroAssets.selector);
        vault.withdraw(0, alice, alice);
    }

    /// @notice withdraw async-claim: no pending request reverts AsyncClaimNoReadyRequest.
    function test_7540_withdraw_claim_noPending_reverts() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AsyncClaimNoReadyRequest.selector);
        vault.withdraw(100e6, alice, alice);
    }

    /// @notice withdraw async-claim: pending amount mismatch reverts.
    function test_7540_withdraw_claim_assetMismatch_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AsyncClaimShareMismatch.selector);
        vault.withdraw(lockedAssets - 1, alice, alice);
    }

    /// @notice withdraw async-claim: pre-cooldown reverts no-ready.
    function test_7540_withdraw_claim_preCooldown_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AsyncClaimNoReadyRequest.selector);
        vault.withdraw(lockedAssets, alice, alice);
    }

    /// @notice withdraw async-claim: paused blocks claim.
    function test_7540_withdraw_claim_paused_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.withdraw(lockedAssets, alice, alice);
    }

    /// @notice redeem async-claim: zero shares reverts ZeroShares.
    function test_7540_redeem_claim_zeroShares_reverts() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroShares.selector);
        vault.redeem(0, alice, alice);
    }

    /// @notice redeem async-claim: paused blocks claim.
    function test_7540_redeem_claim_paused_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.redeem(400e6, alice, alice);
    }

    /// @notice redeem async-claim: zero receiver reverts with ZeroAddress.
    /// Protects: async claim path rejects `receiver == address(0)`, matching
    ///           USDC's own zero-address guard but surfacing a clearer error.
    function test_7540_redeem_claim_zeroReceiver_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroAddress.selector);
        vault.redeem(400e6, address(0), alice);
    }

    /// @notice withdraw async-claim: zero receiver reverts with ZeroAddress.
    function test_7540_withdraw_claim_zeroReceiver_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(reqId);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroAddress.selector);
        vault.withdraw(lockedAssets, address(0), alice);
    }

    /// @notice `redeem` async-claim scan correctly skips claimed requests
    ///         when searching for a matching ready one.
    /// Protects: branch coverage for `req.claimed → continue` in
    ///           `_findReadyRequestByShares`.
    function test_7540_redeem_claim_skipsClaimedRequests() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        // Two requests with identical shares
        vm.prank(alice);
        uint256 r1 = vault.requestRedeem(100e6, alice, alice);
        vm.prank(alice);
        vault.requestRedeem(100e6, alice, alice);

        vm.warp(block.timestamp + COOLDOWN + 1);

        // Claim the first via legacy path
        vm.prank(alice);
        vault.claimWithdraw(r1);

        // Now async claim with the same shares (100e6) — scan must skip r1 and
        // find the second request.
        vm.prank(alice);
        uint256 assets = vault.redeem(100e6, alice, alice);
        assertGt(assets, 0, "7540.C1: second request claimed after skip");
    }

    /// @notice `withdraw` async-claim scan correctly skips claimed requests.
    /// Protects: branch coverage for `req.claimed → continue` in
    ///           `_findReadyRequestByAssets`.
    function test_7540_withdraw_claim_skipsClaimedRequests() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        uint256 r1 = vault.requestRedeem(100e6, alice, alice);
        vm.prank(alice);
        uint256 r2 = vault.requestRedeem(100e6, alice, alice);

        vm.warp(block.timestamp + COOLDOWN + 1);

        // Claim r1 via legacy path
        vm.prank(alice);
        vault.claimWithdraw(r1);

        // r2's locked assets
        (, uint256 lockedAssets,,,) = vault.withdrawRequests(r2);

        // withdraw async claim with matching assets — scan skips r1.
        vm.prank(alice);
        uint256 shares = vault.withdraw(lockedAssets, alice, alice);
        assertEq(shares, 100e6, "7540.C2: second request claimed after skip");
    }

    /// @notice `pendingRedeemRequest` returns 0 for an already-claimed request.
    /// Protects: branch coverage for `req.claimed → return 0` in view.
    function test_7540_pending_returnsZeroForClaimed() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        vm.prank(alice);
        vault.claimWithdraw(reqId);

        assertEq(
            vault.pendingRedeemRequest(reqId, alice), 0, "7540.C3: claimed gives pending 0"
        );
    }

    /// @notice `claimableRedeemRequest` returns 0 for already-claimed request.
    /// Protects: branch coverage for `req.claimed → return 0` in view.
    function test_7540_claimable_returnsZeroForClaimed() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);
        vm.prank(alice);
        uint256 reqId = vault.requestRedeem(400e6, alice, alice);
        vm.warp(block.timestamp + COOLDOWN + 1);
        vm.prank(alice);
        vault.claimWithdraw(reqId);

        assertEq(
            vault.claimableRedeemRequest(reqId, alice),
            0,
            "7540.C4: claimed gives claimable 0"
        );
    }
}
