// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {Dol} from "../src/Dol.sol";
import {pBondJunior} from "../src/pBondJunior.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title pBondJuniorTest
/// @notice Tests for the pBond Junior tranche wrapper and Senior/Junior
///         integration: deposit, redeem, loss absorption, yield waterfall.
contract pBondJuniorTest is Test {
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
    address charlie = makeAddr("charlie");
    uint256 constant COOLDOWN = 86400;

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
            guardian,
            0,
            0
        );

        senior = new Dol(vault, IERC20(address(usdc)), guardian);
        junior = new pBondJunior(vault, IERC20(address(usdc)));

        vm.prank(guardian);
        senior.setJuniorContract(address(junior));
        junior.setSeniorContract(address(senior));

        // Fund test users
        usdc.mint(alice, 100_000e6);
        usdc.mint(bob, 100_000e6);
        usdc.mint(charlie, 100_000e6);

        vm.prank(alice);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(alice);
        usdc.approve(address(junior), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(junior), type(uint256).max);
        vm.prank(charlie);
        usdc.approve(address(senior), type(uint256).max);
        vm.prank(charlie);
        usdc.approve(address(junior), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 11. JUNIOR DEPOSIT — MINTS 1:1
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Depositing USDC into Junior mints pBJ tokens 1:1.
    /// Protects: the 1:1 minting invariant for Junior tranche.
    function test_deposit_mints1to1() public {
        vm.prank(alice);
        junior.deposit(1000e6);

        assertEq(junior.balanceOf(alice), 1000e6, "pBJ minted 1:1");
        assertEq(junior.totalDeposited(), 1000e6, "totalDeposited tracked");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 12. JUNIOR REDEEM — BURNS AND RETURNS USDC
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Full Junior redeem flow: deposit -> redeem -> warp -> claimRedeem.
    /// Protects: the two-step withdraw queue for Junior.
    function test_redeem_burnsAndReturnsUSDC() public {
        vm.prank(alice);
        junior.deposit(1000e6);

        vm.prank(alice);
        uint256 redeemId = junior.redeem(1000e6);
        assertEq(junior.balanceOf(alice), 0, "pBJ burned");

        vm.warp(block.timestamp + COOLDOWN);

        uint256 balBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        junior.claimRedeem(redeemId);
        uint256 received = usdc.balanceOf(alice) - balBefore;
        assertEq(received, 1000e6, "USDC returned in full");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 13. RECEIVE YIELD — ONLY FROM SENIOR
    // ═══════════════════════════════════════════════════════════════════

    /// @notice absorbLoss can only be called by Senior contract.
    /// Protects: access control on the loss absorption function.
    function test_absorbLoss_onlyFromSenior() public {
        vm.prank(alice);
        vm.expectRevert(pBondJunior.OnlySenior.selector);
        junior.absorbLoss(100);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 14. JUNIOR ABSORBS LOSS FIRST
    // ═══════════════════════════════════════════════════════════════════

    /// @notice When the vault loses value, Junior absorbs the loss first
    ///         via distributeYield(), protecting Senior's principal.
    /// Protects: the first-loss buffer guarantee for Senior holders.
    function test_junior_absorbsLossFirst() public {
        // Senior deposits 1000, Junior deposits 1000
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // No NAV report — margin stays 0. totalAssets = idle + treasury = 2000.
        // Drain USDC from vault to simulate loss (vault total drops below deposits)
        uint256 drain = 400e6;
        vm.prank(address(vault));
        usdc.transfer(makeAddr("blackhole"), drain);

        // Warp to allow distribution
        vm.warp(block.timestamp + 1 days);

        uint256 seniorSharesBefore = IERC20(address(vault)).balanceOf(address(senior));
        uint256 juniorSharesBefore = IERC20(address(vault)).balanceOf(address(junior));

        // Distribute — Senior should take shares from Junior
        senior.distributeYield();

        uint256 seniorSharesAfter = IERC20(address(vault)).balanceOf(address(senior));
        uint256 juniorSharesAfter = IERC20(address(vault)).balanceOf(address(junior));

        assertTrue(seniorSharesAfter > seniorSharesBefore, "Senior gained shares from Junior");
        assertTrue(juniorSharesAfter < juniorSharesBefore, "Junior lost shares to Senior");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 15. JUNIOR DEPLETED — SENIOR STARTS LOSING
    // ═══════════════════════════════════════════════════════════════════

    /// @notice When Junior is fully depleted, Senior starts absorbing losses.
    ///         Senior's pricePerShare drops below 1:1.
    /// Protects: the waterfall continues correctly after Junior is wiped.
    function test_junior_depleted_seniorStartsLosing() public {
        // Senior deposits 10000, Junior deposits only 100
        vm.prank(alice);
        senior.deposit(10_000e6);
        vm.prank(bob);
        junior.deposit(100e6);

        // Drain a large amount from vault to simulate heavy loss
        uint256 idle = usdc.balanceOf(address(vault));
        uint256 drain = 2000e6;
        if (drain > idle) drain = idle;
        vm.prank(address(vault));
        usdc.transfer(makeAddr("blackhole"), drain);

        vm.warp(block.timestamp + 1 days);

        // Distribute — Junior can only cover a fraction of the loss
        senior.distributeYield();

        // The deficit was large, Junior may be nearly depleted
        // Senior's price should be below 1:1
        uint256 seniorPrice = senior.pricePerShare();
        assertTrue(seniorPrice < 1e6, "Senior price below 1:1 after Junior depleted");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 16. PRICE PER SHARE — REFLECTS JUNIOR POOL
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Junior's pricePerShare reflects its vault share value.
    /// Protects: accurate pricing for Junior tranche.
    function test_pricePerShare_reflectsJuniorPool() public {
        vm.prank(alice);
        junior.deposit(1000e6);

        uint256 price = junior.pricePerShare();
        assertEq(price, 1e6, "Initial price is 1:1");

        // After NAV increase, Junior's price goes up too
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(200e6, ts, _signNav(200e6, ts));

        uint256 priceAfter = junior.pricePerShare();
        assertTrue(priceAfter > 1e6, "Junior price increased after NAV report");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 17. INTEGRATION — SENIOR AND JUNIOR SHARE SAME VAULT
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Both tranches deposit into the same underlying vault.
    /// Protects: shared vault ownership model.
    function test_seniorAndJunior_shareSameVault() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(500e6);

        uint256 seniorShares = IERC20(address(vault)).balanceOf(address(senior));
        uint256 juniorShares = IERC20(address(vault)).balanceOf(address(junior));
        uint256 totalVaultShares = vault.totalSupply();

        assertTrue(seniorShares > 0, "Senior holds vault shares");
        assertTrue(juniorShares > 0, "Junior holds vault shares");
        assertEq(seniorShares + juniorShares, totalVaultShares, "All vault shares accounted for");
        assertEq(address(senior.vault()), address(junior.vault()), "Same vault reference");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 18. INTEGRATION — YIELD DISTRIBUTION ROUND TRIP
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Full yield cycle: deposit -> NAV increase -> distribute -> verify.
    /// Protects: end-to-end yield waterfall correctness.
    function test_yieldDistribution_roundTrip() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // Report NAV to generate significant yield
        uint256 ts = block.timestamp + 1;
        vault.reportNAV(500e6, ts, _signNav(500e6, ts));

        // Warp 365 days
        vm.warp(block.timestamp + 365 days);

        uint256 totalValueBefore = vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(senior))
        ) + vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(junior))
        );

        senior.distributeYield();

        uint256 totalValueAfter = vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(senior))
        ) + vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(junior))
        );

        // Total value should be conserved (only vault share transfers, no creation/destruction)
        assertApproxEqAbs(totalValueAfter, totalValueBefore, 2, "Total value conserved");

        // Senior should have ~1075e6 (principal + 7.5% yield)
        uint256 seniorValue = vault.convertToAssets(
            IERC20(address(vault)).balanceOf(address(senior))
        );
        assertApproxEqAbs(seniorValue, 1075e6, 2e6, "Senior gets target yield");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 19. INTEGRATION — MULTIPLE DEPOSITORS PRO RATA
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Multiple depositors in the same tranche get pro-rata shares.
    /// Protects: fair share distribution among tranche participants.
    function test_multipleDepositors_proRata() public {
        vm.prank(alice);
        senior.deposit(1000e6);
        vm.prank(bob);
        senior.deposit(2000e6);

        // Alice has 1/3 of Senior shares, Bob has 2/3
        assertEq(senior.balanceOf(alice), 1000e6);
        assertEq(senior.balanceOf(bob), 2000e6);
        assertEq(senior.totalSupply(), 3000e6);

        // Both share the same vault ownership proportionally
        // If Alice redeems her 1000 DOL, she gets 1/3 of vault shares
        vm.prank(alice);
        uint256 redeemId = senior.redeem(1000e6);
        vm.warp(block.timestamp + COOLDOWN);

        uint256 balBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        senior.claimRedeem(redeemId);
        uint256 received = usdc.balanceOf(alice) - balBefore;

        // Alice should get roughly 1/3 of the vault's value at redemption time
        assertApproxEqAbs(received, 1000e6, 1, "Alice gets pro-rata share");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 20. INTEGRATION — LOSS SCENARIO FULL WATERFALL
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Full loss waterfall: vault loses value -> Junior absorbs ->
    ///         Senior is protected up to Junior's capacity.
    /// Protects: the complete waterfall mechanism in a loss scenario.
    function test_lossScenario_fullWaterfall() public {
        // Senior 5000, Junior 1000
        vm.prank(alice);
        senior.deposit(5000e6);
        vm.prank(bob);
        junior.deposit(1000e6);

        // No NAV report — margin stays 0. totalAssets = 6000.
        // Drain vault to simulate a loss (within Junior's capacity)
        uint256 drain = 800e6;
        vm.prank(address(vault));
        usdc.transfer(makeAddr("blackhole"), drain);

        vm.warp(block.timestamp + 1 days);

        uint256 seniorPriceBefore = senior.pricePerShare();

        senior.distributeYield();

        uint256 seniorPriceAfter = senior.pricePerShare();
        uint256 juniorPriceAfter = junior.pricePerShare();

        // Senior should be partially or fully protected
        // Junior should have absorbed loss, reducing its price
        assertTrue(seniorPriceAfter >= seniorPriceBefore,
            "Senior price maintained or improved after loss distribution");
        assertTrue(juniorPriceAfter < 1e6, "Junior price decreased from loss absorption");
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTRA: JUNIOR SET SENIOR ONLY ONCE
    // ═══════════════════════════════════════════════════════════════════

    /// @notice setSeniorContract can only be called once.
    /// Protects: immutability of the Junior-Senior link.
    function test_setSeniorContract_onlyOnce() public {
        vm.expectRevert(pBondJunior.SeniorAlreadySet.selector);
        junior.setSeniorContract(address(senior));
    }

    // ═══════════════════════════════════════════════════════════════════
    // C1 FIX: setSeniorContract ACCESS CONTROL
    // ═══════════════════════════════════════════════════════════════════
    //
    // v1 bug: setSeniorContract had no caller check. Anyone could front-run
    //         the deployer's setup tx and set themselves as "senior", then
    //         call absorbLoss(max) to drain Junior's vault shares.
    //
    // Attack test: deploy a fresh Junior (like a new deployment), then have
    // an attacker front-run the setter. The fix (NotDeployer error) must
    // block this path.

    /// @notice Attacker cannot front-run setSeniorContract.
    /// Protects: C1 — permissionless setter vulnerability.
    function test_c1_setSeniorContract_rejectsNonDeployer() public {
        pBondJunior freshJunior = new pBondJunior(vault, IERC20(address(usdc)));
        address attacker = makeAddr("attacker");

        vm.prank(attacker);
        vm.expectRevert(pBondJunior.NotDeployer.selector);
        freshJunior.setSeniorContract(attacker);
    }

    /// @notice Deployer can set senior exactly once.
    /// Protects: C1 — legitimate setup path still works.
    function test_c1_setSeniorContract_deployerSucceedsOnce() public {
        pBondJunior freshJunior = new pBondJunior(vault, IERC20(address(usdc)));
        address legitSenior = makeAddr("legitSenior");

        // Deployer (this test contract) succeeds
        freshJunior.setSeniorContract(legitSenior);
        assertEq(freshJunior.seniorContract(), legitSenior);

        // Second call reverts with existing "already set" error
        vm.expectRevert(pBondJunior.SeniorAlreadySet.selector);
        freshJunior.setSeniorContract(address(senior));
    }

    /// @notice After deployer sets senior, even deployer cannot re-set.
    /// Protects: C1 — post-setup, senior is effectively immutable.
    function test_c1_setSeniorContract_deployerBlockedAfterSet() public {
        pBondJunior freshJunior = new pBondJunior(vault, IERC20(address(usdc)));
        address legitSenior = makeAddr("legitSenior");
        freshJunior.setSeniorContract(legitSenior);

        // Even the deployer cannot rotate after initial set
        vm.expectRevert(pBondJunior.SeniorAlreadySet.selector);
        freshJunior.setSeniorContract(address(0xdead));
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTRA: JUNIOR REDEEM GUARDS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Junior redeem with insufficient balance reverts.
    function test_junior_redeem_insufficientBalance_reverts() public {
        vm.prank(alice);
        junior.deposit(100e6);

        vm.prank(alice);
        vm.expectRevert(pBondJunior.InsufficientBalance.selector);
        junior.redeem(200e6);
    }

    /// @notice Junior deposit of zero reverts.
    function test_junior_deposit_zero_reverts() public {
        vm.prank(alice);
        vm.expectRevert(pBondJunior.ZeroAmount.selector);
        junior.deposit(0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // COVERAGE-GAP TESTS (2026-04-17 hardening)
    //
    // Branch-coverage targets for pBondJunior: redeem zero, claim-not-owner,
    // claim-double, empty-state pricePerShare.
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Junior redeem with zero amount reverts with ZeroAmount.
    /// Branch: pBondJunior.sol:119
    function test_junior_redeem_zero_reverts() public {
        vm.prank(alice);
        junior.deposit(100e6);

        vm.prank(alice);
        vm.expectRevert(pBondJunior.ZeroAmount.selector);
        junior.redeem(0);
    }

    /// @notice claimRedeem from non-owner reverts with NotRedeemOwner.
    /// Branch: pBondJunior.sol:144
    function test_junior_claimRedeem_notOwner_reverts() public {
        vm.prank(alice);
        junior.deposit(1000e6);
        vm.prank(alice);
        uint256 redeemId = junior.redeem(1000e6);
        vm.warp(block.timestamp + COOLDOWN + 1);

        // Bob attempts to claim alice's redeem
        vm.prank(bob);
        vm.expectRevert(pBondJunior.NotRedeemOwner.selector);
        junior.claimRedeem(redeemId);
    }

    /// @notice Double-claim on Junior reverts with AlreadyClaimed.
    /// Branch: pBondJunior.sol:145
    function test_junior_claimRedeem_doubleClaim_reverts() public {
        vm.prank(alice);
        junior.deposit(1000e6);
        vm.prank(alice);
        uint256 redeemId = junior.redeem(1000e6);
        vm.warp(block.timestamp + COOLDOWN + 1);

        vm.prank(alice);
        junior.claimRedeem(redeemId);

        // Second attempt
        vm.prank(alice);
        vm.expectRevert(pBondJunior.AlreadyClaimed.selector);
        junior.claimRedeem(redeemId);
    }

    /// @notice pricePerShare returns 1e6 when Junior totalSupply is 0.
    /// Branch: pBondJunior.sol:189
    function test_junior_pricePerShare_emptyState() public view {
        // No Junior deposits in setUp
        assertEq(junior.totalSupply(), 0, "Junior supply should be 0");
        assertEq(junior.pricePerShare(), 1e6, "empty Junior price is 1:1");
    }

    /// @notice pBondJunior uses 6 decimals to match USDC.
    function test_junior_decimals_is6() public view {
        assertEq(junior.decimals(), 6, "pBJ decimals must be 6");
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
}
