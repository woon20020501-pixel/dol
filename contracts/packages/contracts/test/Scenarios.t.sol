// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {Dol} from "../src/Dol.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title ScenariosTest
/// @notice End-to-end integration tests that exercise the full user
///         lifecycle: deposit → yield accrual (via NAV report + treasury
///         interest) → redemption (both paths) → fee settlement.
/// @dev These tests deliberately chain multiple state transitions so a
///      regression in any single step shows up here. The assertions check
///      both end-state invariants (user receives expected USDC) AND
///      intermediate invariants (sharePrice monotonic during gains, etc.).
contract ScenariosTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;
    Dol senior;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address charlie = makeAddr("charlie");
    uint256 constant COOLDOWN = 86400;

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);

        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));
        usdc.mint(address(treasury), 10_000_000e6);

        vault = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian, 0, 0
        );
        senior = new Dol(vault, IERC20(address(usdc)), guardian);

        // Fund users
        usdc.mint(alice, 100_000e6);
        usdc.mint(bob, 100_000e6);
        usdc.mint(charlie, 100_000e6);

        vm.prank(alice);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(charlie);
        usdc.approve(address(senior), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-1. Full happy path: deposit → NAV accrual → scheduled redeem
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-1. Alice deposits, operator reports positive NAV, alice
    ///         redeems via the scheduled path and receives USDC > principal.
    /// @dev Validates the core yield delivery mechanism end-to-end.
    ///      Intermediate checks: sharePrice rises after reportNAV, Dol
    ///      balance stays constant (non-rebasing), scheduled cooldown
    ///      is enforced strictly.
    function test_scenario_S1_happyPath_deposit_nav_redeem() public {
        // --- Deposit phase ---
        vm.prank(alice);
        senior.deposit(10_000e6);
        assertEq(senior.balanceOf(alice), 10_000e6, "S1.1: Dol minted 1:1");
        assertEq(usdc.balanceOf(alice), 90_000e6, "S1.2: USDC moved");
        assertEq(senior.pricePerShare(), 1e6, "S1.3: initial PPS = 1:1");

        // --- NAV report (simulate +2% over time from off-chain margin gain) ---
        uint256 ts1 = block.timestamp + 1;
        // totalAssetsStored represents off-chain margin; we report +200 USDC gain
        uint256 newNav = 200e6;
        vm.warp(ts1);
        _reportNav(newNav, ts1);
        assertGt(senior.pricePerShare(), 1e6, "S1.4: PPS rose after NAV gain");

        // --- Scheduled redeem ---
        uint256 aliceDolBefore = senior.balanceOf(alice);
        vm.prank(alice);
        uint256 redeemId = senior.redeem(aliceDolBefore);
        assertEq(senior.balanceOf(alice), 0, "S1.5: all Dol burned");

        // --- Cooldown enforcement ---
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.CooldownNotElapsed.selector);
        senior.claimRedeem(redeemId);

        // --- Simulate operator closing off-chain positions and injecting USDC
        //     back to vault (what the bot does when it unwinds for a claim).
        //     Without this, NAV-inflated claims can't be physically paid — the
        //     totalAssetsStored slot is synthetic until converted.
        usdc.mint(address(vault), 200e6);

        // --- Claim after cooldown ---
        vm.warp(block.timestamp + COOLDOWN + 1);
        uint256 aliceUsdcBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        senior.claimRedeem(redeemId);
        uint256 received = usdc.balanceOf(alice) - aliceUsdcBefore;
        assertGe(received, 10_000e6, "S1.6: alice recovered >= principal");
        // Alice's share of +200 NAV gain (she's the only depositor) ~= 200
        // (minus treasury rounding). Allow up to +210 for treasury drift.
        assertLe(received, 10_210e6, "S1.7: alice received bounded yield");
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-2. Multi-user pro-rata with staggered entry
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-2. Bob depositing after a yield event receives PROPORTIONALLY
    ///         FEWER vault shares (so he cannot dilute Alice's pre-entry
    ///         gain). This asserts the core pro-rata fairness invariant at
    ///         the share-accounting layer, independent of redemption path
    ///         or treasury drift.
    function test_scenario_S2_multiUser_proRata_staggered() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        // Simulate yield as a realized USDC inflow (operator unwound +10 USDC
        // of off-chain carry into the vault's idle bucket). This increases
        // totalAssets live without touching totalAssetsStored, which is the
        // correct end-state after a close-position flow.
        usdc.mint(address(vault), 10e6);

        uint256 ppsAfterAliceOnly = senior.pricePerShare();
        assertGt(ppsAfterAliceOnly, 1e6, "S2.1: Alice captured solo yield");

        // Snapshot Alice's vault-share holding (via Dol's balance) before Bob
        uint256 dolVaultSharesBeforeBob = vault.balanceOf(address(senior));

        // Bob enters at the new higher PPS. His Dol balance is 1:1 (1000 Dol)
        // but his IMPLICIT vault-share claim is proportional to pre-Bob PPS.
        vm.prank(bob);
        senior.deposit(1000e6);
        assertEq(senior.balanceOf(bob), 1000e6, "S2.2: Bob gets 1:1 Dol");

        uint256 dolVaultSharesAfterBob = vault.balanceOf(address(senior));
        uint256 bobContributedShares = dolVaultSharesAfterBob - dolVaultSharesBeforeBob;

        // CORE INVARIANT: Bob's 1000 USDC bought him FEWER vault shares than
        // Alice's 1000 USDC did (because PPS rose between the two deposits).
        // This is the share-accounting layer's pro-rata fairness: Alice's
        // pre-entry yield is reflected in her higher share ratio per Dol,
        // not in Bob's claim.
        assertLt(
            bobContributedShares,
            dolVaultSharesBeforeBob,
            "S2.3: Bob's 1000 USDC < Alice's 1000 USDC in vault shares"
        );

        // Alice's Dol is backed by MORE vault shares per Dol than Bob's.
        // Compute the per-Dol backing ratio for each:
        //   aliceShares : 1000 Dol   → aliceShares / 1000 shares per Dol
        //   bobContributedShares : 1000 Dol  → bobContributedShares / 1000 per Dol
        assertGt(
            dolVaultSharesBeforeBob, // Alice owned this before Bob entered
            bobContributedShares,
            "S2.4: Alice's per-Dol backing > Bob's per-Dol backing"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-3. Pause during operation — requestWithdraw still allowed
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-3. Users can always exit-signal during pause, claims
    ///         enforce the pause gate (current design).
    function test_scenario_S3_pauseDoesNotBlockRequestWithdraw() public {
        vm.prank(alice);
        senior.deposit(1000e6);

        vm.prank(guardian);
        vault.pause();

        // New deposit blocked
        vm.prank(bob);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        senior.deposit(1000e6);

        // But alice can still request exit (burn shares + queue)
        uint256 aliceDol = senior.balanceOf(alice);
        vm.prank(alice);
        uint256 redeemId = senior.redeem(aliceDol);
        assertEq(senior.balanceOf(alice), 0, "S3.1: Dol burned while paused");

        // Claim during pause currently reverts (see C4 in AUDIT_PREP).
        // This test pins the current behavior so a future fix is surfaced
        // by a regression diff, not hidden in silent behavior change.
        vm.warp(block.timestamp + COOLDOWN + 1);
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        senior.claimRedeem(redeemId);

        // Unpause → claim succeeds
        vm.prank(guardian);
        vault.unpause();
        vm.prank(alice);
        senior.claimRedeem(redeemId);
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-4. Instant-path fee routing
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-4. 5 bps fee on instantRedeem flows to feeRecipient and
    ///         user receives exactly `gross - fee` USDC.
    function test_scenario_S4_instantFee_routing() public {
        vm.prank(alice);
        senior.deposit(10_000e6);

        uint256 feeRecipientBefore = usdc.balanceOf(guardian);
        uint256 aliceBefore = usdc.balanceOf(alice);

        // Redeem half via instant path
        vm.prank(alice);
        uint256 out = senior.instantRedeem(5000e6);

        uint256 expectedFee = (5000e6 * 5) / 10_000; // 5 bps on notional
        uint256 expectedNet = 5000e6 - expectedFee;
        assertEq(out, expectedNet, "S4.1: caller receives net");
        assertEq(usdc.balanceOf(alice) - aliceBefore, expectedNet, "S4.2: alice net USDC");
        assertEq(
            usdc.balanceOf(guardian) - feeRecipientBefore,
            expectedFee,
            "S4.3: feeRecipient got fee"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-5. NAV sanity guard regression: <=10% blocked at exactly 10%
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-5. Reports exactly 10% apart from lastNav must revert
    ///         (strict `>` guard). 9.99% accepted; 10.00% rejected.
    function test_scenario_S5_navGuard_boundary() public {
        vm.prank(alice);
        senior.deposit(10_000e6);

        uint256 ts1 = block.timestamp + 1;
        vm.warp(ts1);
        _reportNav(10_000e6, ts1); // baseline

        // Exact 10% → revert
        uint256 ts2 = ts1 + 1;
        vm.warp(ts2);
        uint256 badNav = 11_000e6;
        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(badNav, ts2, _signNav(badNav, ts2));

        // 9.99% → success
        uint256 goodNav = 10_999e6;
        _reportNav(goodNav, ts2);
        assertEq(vault.totalAssetsStored(), goodNav, "S5.1: near-boundary accepted");
    }

    // ═══════════════════════════════════════════════════════════════════
    // S-6. Long-horizon treasury interest accrual
    // ═══════════════════════════════════════════════════════════════════

    /// @notice S-6. Over a 1-year horizon, the 30% treasury allocation
    ///         earns MockMoonwell's 5% APY, producing a small but
    ///         positive sharePrice drift even without reportNAV.
    function test_scenario_S6_treasuryInterest_oneYear() public {
        vm.prank(alice);
        senior.deposit(10_000e6); // 7000 idle + 3000 treasury

        uint256 ppsStart = senior.pricePerShare();

        // Warp 1 year
        vm.warp(block.timestamp + 365 days);

        uint256 ppsEnd = senior.pricePerShare();
        // Expected: treasury 3000 × 5% = 150 USDC interest over 10000 principal = +1.5% PPS
        uint256 expectedMinPpsGainBps = 140; // 1.40% — below exact to tolerate rounding
        uint256 expectedMaxPpsGainBps = 160;
        uint256 actualGainBps = ((ppsEnd - ppsStart) * 10_000) / ppsStart;
        assertGe(actualGainBps, expectedMinPpsGainBps, "S6.1: >= +1.40% after 1y");
        assertLe(actualGainBps, expectedMaxPpsGainBps, "S6.2: <= +1.60% after 1y");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Helpers
    // ═══════════════════════════════════════════════════════════════════

    function _reportNav(uint256 newNav, uint256 timestamp) internal {
        vault.reportNAV(newNav, timestamp, _signNav(newNav, timestamp));
    }

    function _signNav(uint256 newNav, uint256 timestamp) internal view returns (bytes memory) {
        bytes32 payloadHash = keccak256(
            abi.encodePacked("PACIFICA_CARRY_VAULT_NAV", address(vault), newNav, timestamp)
        );
        bytes32 ethHash = keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", payloadHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(OPERATOR_PK, ethHash);
        return abi.encodePacked(r, s, v);
    }
}
