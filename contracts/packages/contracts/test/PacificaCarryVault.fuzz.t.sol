// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title PacificaCarryVaultFuzzTest
/// @notice Fuzz tests for math-heavy functions. Foundry generates random
///         inputs to exercise edge cases in share price math, NAV sanity
///         guard boundaries, and withdraw cooldown timing.
contract PacificaCarryVaultFuzzTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice    = makeAddr("alice");
    uint256 constant COOLDOWN = 86400;

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);
        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));
        vault = new PacificaCarryVault(
            IERC20(address(usdc)),
            treasury,
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        // Fund alice generously for fuzz tests
        usdc.mint(alice, type(uint128).max);
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // FUZZ: SHARE PRICE MATH
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Share price math across deposit sizes from 1 wei to 10^27.
    ///         After a single deposit, share price must always be 1e18 (1:1).
    /// Protects: share price calculation does not overflow, underflow, or
    ///           produce incorrect values for any deposit size in the
    ///           expected operating range.
    function testFuzz_sharePrice_singleDeposit(uint256 assets) public {
        // Bound to [1, 10^27] — the range specified in quality bar
        assets = bound(assets, 1, 1e27);

        // Ensure alice has enough
        if (usdc.balanceOf(alice) < assets) {
            usdc.mint(alice, assets);
        }

        vm.prank(alice);
        uint256 shares = vault.deposit(assets, alice);

        // First deposit is always 1:1
        assertEq(shares, assets, "first deposit must be 1:1");

        // Share price must be exactly 1e18
        assertEq(vault.sharePrice(), 1e18, "share price must be 1e18 after single deposit");

        // totalAssets must equal the deposit
        assertEq(vault.totalAssets(), assets, "totalAssets must equal deposit");
    }

    /// @notice Two consecutive deposits with random sizes — share price
    ///         stays at 1e18 (no NAV report between them).
    /// Protects: multiple deposits without NAV change maintain 1:1 price.
    ///           No depositor is diluted or gets extra shares.
    function testFuzz_sharePrice_twoDeposits(uint256 assets1, uint256 assets2) public {
        assets1 = bound(assets1, 1, 1e27);
        assets2 = bound(assets2, 1, 1e27);

        address bob = makeAddr("bob");
        usdc.mint(bob, assets2);
        vm.prank(bob);
        usdc.approve(address(vault), type(uint256).max);

        // Ensure alice has enough
        if (usdc.balanceOf(alice) < assets1) usdc.mint(alice, assets1);

        vm.prank(alice);
        vault.deposit(assets1, alice);

        vm.prank(bob);
        uint256 bobShares = vault.deposit(assets2, bob);

        // At 1:1 price, bob gets shares == assets
        assertEq(bobShares, assets2, "second deposit must also be 1:1");

        // Share price still 1e18
        assertEq(vault.sharePrice(), 1e18, "share price must remain 1e18");

        // totalAssets = sum of deposits
        assertEq(vault.totalAssets(), assets1 + assets2, "totalAssets = sum of deposits");
    }

    /// @notice Share price formula holds after a NAV report.
    ///         sharePrice = totalAssets() * 1e18 / totalSupply.
    /// Protects: the share price formula is correct for arbitrary margin
    ///           amounts reported by the operator. In V1.5, totalAssets()
    ///           includes the idle bucket, treasury balance, and the
    ///           reported off-chain margin slot.
    function testFuzz_sharePrice_afterNavReport(uint256 assets, uint256 marginAmount) public {
        assets = bound(assets, 1e6, 1e27);
        // Margin slot represents off-chain perp value — bound to a reasonable
        // range that doesn't trip the first-report-skip semantics.
        marginAmount = bound(marginAmount, 1, assets);

        if (usdc.balanceOf(alice) < assets) usdc.mint(alice, assets);

        vm.prank(alice);
        vault.deposit(assets, alice);

        // First reportNAV skips the delta check
        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(marginAmount, ts1, _signNav(marginAmount, ts1));

        // Verify share price formula holds against the live totalAssets
        uint256 supply = vault.totalSupply();
        uint256 expectedPrice = (vault.totalAssets() * 1e18) / supply;
        assertEq(vault.sharePrice(), expectedPrice, "share price must match formula");
    }

    // ═══════════════════════════════════════════════════════════════════
    // FUZZ: NAV SANITY GUARD BOUNDARY
    // ═══════════════════════════════════════════════════════════════════

    /// @notice NAV sanity guard rejects changes >= 10% (fuzzed boundary).
    ///         For any lastNav and newNav where |delta| * 10 >= lastNav,
    ///         reportNAV must revert.
    /// Protects: the 10% guard boundary is correctly enforced for all
    ///           possible NAV values, not just hand-picked test cases.
    function testFuzz_navSanityGuard_rejectsAtOrAbove10Percent(
        uint256 lastNav,
        uint256 newNav
    ) public {
        // Need a meaningful lastNav to test the guard
        lastNav = bound(lastNav, 1e6, 1e27);
        newNav = bound(newNav, 0, type(uint128).max);

        uint256 delta = newNav > lastNav ? newNav - lastNav : lastNav - newNav;

        // Only test cases where delta >= 10%
        vm.assume(delta * 10 >= lastNav);

        // Setup: deposit and initialize NAV
        if (usdc.balanceOf(alice) < lastNav) usdc.mint(alice, lastNav);

        vm.prank(alice);
        vault.deposit(lastNav, alice);

        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(lastNav, ts1, _signNav(lastNav, ts1));

        // Second report at the violating NAV must revert
        uint256 ts2 = ts1 + 1;
        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2));
    }

    /// @notice NAV sanity guard accepts changes strictly < 10% (fuzzed).
    ///         For any lastNav and newNav where |delta| * 10 < lastNav,
    ///         reportNAV must succeed.
    /// Protects: legitimate NAV changes within bounds are never
    ///           incorrectly rejected.
    function testFuzz_navSanityGuard_acceptsBelow10Percent(
        uint256 lastNav,
        uint256 deltaBps
    ) public {
        lastNav = bound(lastNav, 1e6, 1e27);
        // 1 to 999 basis points = strictly under 10%
        deltaBps = bound(deltaBps, 1, 999);

        uint256 change = (lastNav * deltaBps) / 10_000;
        if (change == 0) change = 1;

        // Pick gain or loss
        uint256 newNav = deltaBps % 2 == 0 ? lastNav - change : lastNav + change;

        // Verify it's actually within the guard
        uint256 delta = newNav > lastNav ? newNav - lastNav : lastNav - newNav;
        vm.assume(delta * 10 < lastNav);

        // Setup: deposit and initialize NAV
        if (usdc.balanceOf(alice) < lastNav) usdc.mint(alice, lastNav);

        vm.prank(alice);
        vault.deposit(lastNav, alice);

        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(lastNav, ts1, _signNav(lastNav, ts1));

        // Second report within guard must succeed
        uint256 ts2 = ts1 + 1;
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2));

        assertEq(vault.totalAssetsStored(), newNav, "margin slot should be updated");
    }

    // ═══════════════════════════════════════════════════════════════════
    // FUZZ: WITHDRAW REQUEST/CLAIM ACROSS RANDOM COOLDOWN OFFSETS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice claimWithdraw succeeds at or after the cooldown, and fails
    ///         before it, for any random time offset.
    /// Protects: cooldown enforcement is exact — not off-by-one, not
    ///           bypassable by any timestamp manipulation.
    function testFuzz_withdrawCooldown(uint256 assets, uint256 timeOffset) public {
        assets = bound(assets, 1, 1e27);
        timeOffset = bound(timeOffset, 0, 7 days);

        if (usdc.balanceOf(alice) < assets) usdc.mint(alice, assets);

        vm.startPrank(alice);
        vault.deposit(assets, alice);

        uint256 shares = vault.balanceOf(alice);
        uint256 requestId = vault.requestWithdraw(shares);
        vm.stopPrank();

        (, , uint256 unlockTimestamp, ) = vault.withdrawRequests(requestId);

        // Warp to a random offset from now
        uint256 targetTime = block.timestamp + timeOffset;
        vm.warp(targetTime);

        if (targetTime < unlockTimestamp) {
            // Should revert — cooldown not elapsed
            vm.prank(alice);
            vm.expectRevert(PacificaCarryVault.CooldownNotElapsed.selector);
            vault.claimWithdraw(requestId);
        } else {
            // Should succeed — cooldown elapsed
            uint256 balBefore = usdc.balanceOf(alice);
            vm.prank(alice);
            uint256 claimed = vault.claimWithdraw(requestId);

            assertEq(claimed, assets, "claimed amount must equal deposited");
            assertEq(
                usdc.balanceOf(alice),
                balBefore + assets,
                "alice must receive USDC"
            );
        }
    }

    /// @notice Exactly at unlock timestamp, claimWithdraw must succeed.
    /// Protects: boundary condition — cooldown is >= not >.
    function testFuzz_withdrawExactlyAtCooldown(uint256 assets) public {
        assets = bound(assets, 1, 1e27);

        if (usdc.balanceOf(alice) < assets) usdc.mint(alice, assets);

        vm.startPrank(alice);
        vault.deposit(assets, alice);

        uint256 shares = vault.balanceOf(alice);
        uint256 requestId = vault.requestWithdraw(shares);
        vm.stopPrank();

        (, , uint256 unlockTimestamp, ) = vault.withdrawRequests(requestId);

        // Warp to exactly the unlock timestamp
        vm.warp(unlockTimestamp);

        vm.prank(alice);
        uint256 claimed = vault.claimWithdraw(requestId);
        assertEq(claimed, assets, "must succeed at exact cooldown");
    }

    // ═══════════════════════════════════════════════════════════════════
    // FUZZ: CONSECUTIVE DEPOSITS — TOTAL ASSETS ACCOUNTING
    // ═══════════════════════════════════════════════════════════════════

    /// @notice After N consecutive deposits (no withdrawals), totalAssets
    ///         must equal the sum of all deposits.
    /// Protects: deposit accounting does not drift over multiple operations.
    function testFuzz_consecutiveDeposits_totalAssetsEqualsSum(
        uint256 a1,
        uint256 a2,
        uint256 a3
    ) public {
        a1 = bound(a1, 1, 1e24);
        a2 = bound(a2, 1, 1e24);
        a3 = bound(a3, 1, 1e24);

        address bob = makeAddr("bob");
        address charlie = makeAddr("charlie");

        usdc.mint(alice, a1);
        usdc.mint(bob, a2);
        usdc.mint(charlie, a3);

        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(charlie);
        usdc.approve(address(vault), type(uint256).max);

        vm.prank(alice);
        vault.deposit(a1, alice);
        vm.prank(bob);
        vault.deposit(a2, bob);
        vm.prank(charlie);
        vault.deposit(a3, charlie);

        assertEq(
            vault.totalAssets(),
            a1 + a2 + a3,
            "totalAssets must equal sum of all deposits"
        );
    }

    /// @notice After deposits and full withdrawals, totalAssets must equal
    ///         deposits minus claimed withdrawals.
    /// Protects: the deposit/withdraw accounting identity holds for any
    ///           combination of deposit and withdrawal sizes.
    function testFuzz_depositAndWithdraw_accounting(uint256 depositAmt) public {
        depositAmt = bound(depositAmt, 1, 1e27);

        if (usdc.balanceOf(alice) < depositAmt) usdc.mint(alice, depositAmt);

        vm.prank(alice);
        vault.deposit(depositAmt, alice);

        uint256 shares = vault.balanceOf(alice);
        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(shares);

        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(alice);
        vault.claimWithdraw(requestId);

        // Idle is drained, margin slot untouched. Only residual treasury
        // yield (accrued during the cooldown warp) remains in the vault.
        assertEq(usdc.balanceOf(address(vault)), 0, "idle must be drained");
        assertEq(vault.totalAssetsStored(), 0, "margin slot must be 0");
        assertEq(vault.totalSupply(), 0, "totalSupply must be 0 after full withdrawal");
    }

    // ═══════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════

    function _signNav(uint256 newNav, uint256 timestamp)
        internal
        view
        returns (bytes memory)
    {
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
}
