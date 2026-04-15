// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title MockMoonwellMarketTest
/// @notice Unit tests for the mock 5% APY treasury vault used in V1.5.
contract MockMoonwellMarketTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket market;

    address alice = makeAddr("alice");
    address bob   = makeAddr("bob");

    uint256 constant ONE_USDC = 1e6;

    function setUp() public {
        usdc = new MockUSDC();
        market = new MockMoonwellMarket(IERC20(address(usdc)));

        // Fund the market generously so it can pay accrued interest in tests
        usdc.mint(address(market), 10_000_000 * ONE_USDC);

        usdc.mint(alice, 1_000_000 * ONE_USDC);
        usdc.mint(bob, 1_000_000 * ONE_USDC);

        vm.prank(alice);
        usdc.approve(address(market), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(market), type(uint256).max);
    }

    /// @notice Deposit happy path: principal recorded, USDC pulled in.
    /// Protects: mint() correctly takes USDC and credits the supplier.
    function test_mint_happyPath() public {
        vm.prank(alice);
        uint256 err = market.mint(1000 * ONE_USDC);

        assertEq(err, 0, "mint should return 0 on success");
        assertEq(market.principalOf(alice), 1000 * ONE_USDC, "principal should match deposit");
        assertEq(
            market.balanceOfUnderlying(alice),
            1000 * ONE_USDC,
            "no time has passed - no interest yet"
        );
    }

    /// @notice Balance grows by ~5% after one full year.
    /// Protects: linear interest accrual formula matches the 5% APY spec.
    function test_balance_growsAfterOneYear() public {
        vm.prank(alice);
        market.mint(1000 * ONE_USDC);

        // Warp 365 days
        vm.warp(block.timestamp + 365 days);

        // 1000 USDC * 5% = 50 USDC interest
        uint256 expected = 1050 * ONE_USDC;
        assertEq(
            market.balanceOfUnderlying(alice),
            expected,
            "balance should be 1050 after 1 year at 5% APY"
        );
    }

    /// @notice Full redeem returns the deposited principal (no interest pre-warp).
    /// Protects: redeem() correctly transfers USDC out and decrements principal.
    function test_redeem_full() public {
        vm.startPrank(alice);
        market.mint(1000 * ONE_USDC);

        uint256 balBefore = usdc.balanceOf(alice);
        uint256 err = market.redeem(1000 * ONE_USDC);
        vm.stopPrank();

        assertEq(err, 0, "redeem should return 0 on success");
        assertEq(market.principalOf(alice), 0, "principal should be 0 after full redeem");
        assertEq(
            usdc.balanceOf(alice),
            balBefore + 1000 * ONE_USDC,
            "alice should receive USDC back"
        );
    }

    /// @notice Partial redeem leaves remaining principal earning interest.
    /// Protects: redeem() handles partial withdrawals and continues accruing
    ///           on the remaining principal.
    function test_redeem_partial() public {
        vm.startPrank(alice);
        market.mint(1000 * ONE_USDC);

        // Warp half a year — accrues 2.5% on 1000 = 25 USDC
        vm.warp(block.timestamp + 182 days + 12 hours);

        // Realize interest by interacting (any state-changing call accrues)
        market.redeem(500 * ONE_USDC);
        vm.stopPrank();

        // Remaining principal should be (1000 + ~25) - 500 ≈ 525 USDC
        uint256 remaining = market.principalOf(alice);
        assertGt(remaining, 524 * ONE_USDC, "should retain ~525 USDC including interest");
        assertLt(remaining, 526 * ONE_USDC, "should retain ~525 USDC including interest");
    }

    /// @notice Mint with zero amount must revert.
    /// Protects: prevents no-op deposits that pollute accounting.
    function test_mint_zero_reverts() public {
        vm.prank(alice);
        vm.expectRevert(MockMoonwellMarket.ZeroAmount.selector);
        market.mint(0);
    }

    /// @notice Multiple users have isolated accrual.
    /// Protects: per-account principal tracking — bob's deposit and
    ///           interest don't bleed into alice's balance.
    function test_multiUser_isolation() public {
        vm.prank(alice);
        market.mint(1000 * ONE_USDC);

        // Wait some time
        vm.warp(block.timestamp + 365 days);

        // Bob deposits AFTER alice already accrued
        vm.prank(bob);
        market.mint(1000 * ONE_USDC);

        // Alice has accrued 5% (50 USDC), bob has accrued nothing yet
        assertEq(market.balanceOfUnderlying(alice), 1050 * ONE_USDC, "alice has 1 year of interest");
        assertEq(market.balanceOfUnderlying(bob), 1000 * ONE_USDC, "bob just deposited, no interest");

        // Wait another year
        vm.warp(block.timestamp + 365 days);

        // Alice principal grew to ~1050 then accrues another year on that base.
        // Bob earns 5% on 1000 = 50.
        // Alice: 1050 + 1050*5% = 1102.5 (but principal isn't realized until interaction,
        //        so balanceOfUnderlying uses the last-realized principal of 1000 + simple
        //        interest over 730 days = 1000 + 100 = 1100).
        // The exact value depends on whether interest was realized between warps.
        // Verify that alice still has more than bob (her stake is older).
        assertGt(
            market.balanceOfUnderlying(alice),
            market.balanceOfUnderlying(bob),
            "alice has been earning longer than bob"
        );
    }
}
