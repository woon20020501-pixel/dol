// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title PacificaCarryVaultNavReportTest
/// @notice End-to-end integration tests that exercise the NAV-report signing
///         flow exactly the way the off-chain bot does. Each test constructs
///         the signing payload from raw fields, hashes with the EIP-191
///         prefix, signs with a known operator private key, and submits via
///         `reportNAV`. These tests serve as the reference the bot mirrors —
///         if any test in this file breaks, the bot's signer is also broken.
contract PacificaCarryVaultNavReportTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;

    // Deterministic operator key so the bot can reproduce signatures locally.
    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;

    address guardian = makeAddr("guardian");
    address alice    = makeAddr("alice");

    uint256 constant COOLDOWN = 86400;
    uint256 constant SEED_DEPOSIT = 1_000e6;

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

        // Seed the vault so totalAssetsStored has a baseline for the
        // sanity-guard scenarios. Alice deposits then we initialize the
        // margin slot to a known value via a first-time report.
        usdc.mint(alice, SEED_DEPOSIT);
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(alice);
        vault.deposit(SEED_DEPOSIT, alice);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 1. VALID SIGNATURE — END-TO-END HAPPY PATH
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Operator signs a valid NAV update; vault accepts and updates state.
    /// Protects: the full signing flow the bot runs — payload encoding,
    ///           EIP-191 prefix, ECDSA signing, contract recovery — must
    ///           round-trip without any byte-level discrepancy.
    function test_reportNAV_validSignature_succeeds() public {
        uint256 newNav = 500e6; // arbitrary margin slot value (USDC, 6 dec)
        uint256 ts = block.timestamp + 60;

        bytes memory sig = _signNav(newNav, ts, OPERATOR_PK);

        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.NavReported(newNav, ts);
        vault.reportNAV(newNav, ts, sig);

        assertEq(vault.totalAssetsStored(), newNav, "margin slot updated");
        assertEq(vault.lastTimestamp(), ts, "timestamp recorded");
        assertTrue(vault.navInitialized(), "navInitialized flag set");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 2. INVALID SIGNER — NON-OPERATOR KEY REJECTED
    // ═══════════════════════════════════════════════════════════════════

    /// @notice A signature minted by any key other than the current operator
    ///         is rejected with InvalidNavSignature, even if the payload is
    ///         otherwise valid.
    /// Protects: the operator role check is enforced via ECDSA recovery,
    ///           not just an `onlyRole` modifier on msg.sender. The bot key
    ///           is the only key that can move the oracle.
    function test_reportNAV_invalidSigner_reverts() public {
        uint256 attackerPk = 0xBADBAD;
        uint256 newNav = 500e6;
        uint256 ts = block.timestamp + 60;

        bytes memory badSig = _signNav(newNav, ts, attackerPk);

        vm.expectRevert(PacificaCarryVault.InvalidNavSignature.selector);
        vault.reportNAV(newNav, ts, badSig);
    }

    // ═══════════════════════════════════════════════════════════════════
    // 3. SANITY GUARD — DROP > 10% REJECTED
    // ═══════════════════════════════════════════════════════════════════

    /// @notice After a baseline NAV is established, a single report that
    ///         drops the margin slot by more than 10% is rejected.
    /// Protects: a compromised operator key cannot zero out the vault in
    ///           one report; the worst-case damage per report is bounded.
    function test_reportNAV_sanityGuardLow_reverts() public {
        // Establish baseline at 1000e6
        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(1_000e6, ts1, _signNav(1_000e6, ts1, OPERATOR_PK));

        // Try to drop to 850e6 = -15%. delta = 150, 150 * 10 = 1500 >= 1000 → revert.
        uint256 ts2 = ts1 + 1;
        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(850e6, ts2, _signNav(850e6, ts2, OPERATOR_PK));
    }

    // ═══════════════════════════════════════════════════════════════════
    // 4. SANITY GUARD — RISE > 10% REJECTED (SYMMETRY)
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Symmetric to the drop case: a single report that lifts the
    ///         margin slot by more than 10% is also rejected.
    /// Protects: the guard is two-sided. A compromised key cannot inflate
    ///           the share price arbitrarily and steal value via a withdraw.
    function test_reportNAV_sanityGuardHigh_reverts() public {
        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(1_000e6, ts1, _signNav(1_000e6, ts1, OPERATOR_PK));

        // Try to rise to 1150e6 = +15%. delta = 150, 150 * 10 = 1500 >= 1000 → revert.
        uint256 ts2 = ts1 + 1;
        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(1_150e6, ts2, _signNav(1_150e6, ts2, OPERATOR_PK));
    }

    // ═══════════════════════════════════════════════════════════════════
    // 5. FIRST REPORT — SKIPS THE DELTA GUARD
    // ═══════════════════════════════════════════════════════════════════

    /// @notice The very first NAV report after deployment skips the delta
    ///         check entirely (there is no prior value to compare against).
    /// Protects: the contract correctly bootstraps the oracle. Without this
    ///           skip, the very first report would always revert because
    ///           lastNav == 0 makes any non-zero delta exceed the guard.
    function test_reportNAV_firstReport_skipsGuard() public {
        // navInitialized must start false — confirmed in setUp's assertion
        // domain. We have not yet called reportNAV in this test.
        assertFalse(vault.navInitialized(), "guard skip only applies to first report");

        // Pick an arbitrary value far from any baseline. With navInitialized
        // false the contract must accept it without delta math.
        uint256 newNav = 7_777e6;
        uint256 ts = block.timestamp + 1;

        vault.reportNAV(newNav, ts, _signNav(newNav, ts, OPERATOR_PK));

        assertEq(vault.totalAssetsStored(), newNav, "first report establishes baseline");
        assertTrue(vault.navInitialized(), "navInitialized flips to true");
    }

    // ═══════════════════════════════════════════════════════════════════
    // 6. REPLAY PROTECTION — TIMESTAMP MONOTONICITY
    // ═══════════════════════════════════════════════════════════════════

    /// @notice A previously-accepted NAV report cannot be replayed. The
    ///         contract enforces strict timestamp monotonicity:
    ///         `timestamp > lastTimestamp`. Any replay (same timestamp or
    ///         older) reverts with StaleTimestamp.
    /// Protects: an attacker cannot capture a valid signature off the wire
    ///           and resubmit it later, even if they convince the operator
    ///           to sign the same nav at the same timestamp twice.
    function test_reportNAV_replayProtection() public {
        uint256 nav = 1_000e6;
        uint256 ts1 = block.timestamp + 100;

        bytes memory sig = _signNav(nav, ts1, OPERATOR_PK);

        // First submission succeeds
        vault.reportNAV(nav, ts1, sig);
        assertEq(vault.lastTimestamp(), ts1, "first submission accepted");

        // Replay with the SAME bytes — must revert (timestamp is stale now)
        vm.expectRevert(PacificaCarryVault.StaleTimestamp.selector);
        vault.reportNAV(nav, ts1, sig);

        // Even resigning at an older timestamp must revert
        uint256 olderTs = ts1 - 50;
        bytes memory olderSig = _signNav(nav, olderTs, OPERATOR_PK);
        vm.expectRevert(PacificaCarryVault.StaleTimestamp.selector);
        vault.reportNAV(nav, olderTs, olderSig);
    }

    // ═══════════════════════════════════════════════════════════════════
    // GOLDEN VECTOR — REFERENCE BYTES FOR THE OFF-CHAIN SIGNER
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Emits a deterministic golden vector to stdout. The off-chain
    ///         signer must produce byte-for-byte identical output for the
    ///         same inputs. Run with `forge test --match-test test_goldenVector_emit -vv`
    ///         to see the values logged.
    /// @dev Inputs are fixed constants so the output is reproducible across
    ///      runs. Operator key 0xA11CE is the same key used in every test
    ///      file in this package.
    function test_goldenVector_emit() public view {
        // Fixed inputs — change here only if INTERFACES.md is updated.
        address fixedVault = 0xD08C1C78E3Fc6Ac007C06F2b73a28eA8b057A522;
        uint256 fixedNav = 1_000_000_000; // 1000 USDC, 6 decimals
        uint256 fixedTimestamp = 1_775_000_000; // unix seconds (2026-04-26)
        uint256 fixedPk = OPERATOR_PK;

        // Step 1: encodePacked of the four fields
        bytes memory packed = abi.encodePacked(
            "PACIFICA_CARRY_VAULT_NAV",
            fixedVault,
            fixedNav,
            fixedTimestamp
        );

        // Step 2: keccak256 of the packed bytes (the inner payload hash)
        bytes32 payloadHash = keccak256(packed);

        // Step 3: EIP-191 prefix and re-hash
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", payloadHash)
        );

        // Step 4: ECDSA signature (r, s, v) using the operator key
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(fixedPk, ethSignedHash);
        bytes memory signature = abi.encodePacked(r, s, v);

        console.log("=== NAV REPORT GOLDEN VECTOR ===");
        console.log("operator address:", vm.addr(fixedPk));
        console.log("vault address:   ", fixedVault);
        console.log("nav:             ", fixedNav);
        console.log("timestamp:       ", fixedTimestamp);
        console.log("packed bytes:");
        console.logBytes(packed);
        console.log("inner payload hash:");
        console.logBytes32(payloadHash);
        console.log("eth-signed message hash:");
        console.logBytes32(ethSignedHash);
        console.log("signature (r || s || v):");
        console.logBytes(signature);
        console.log("v:", v);
        console.log("r:");
        console.logBytes32(r);
        console.log("s:");
        console.logBytes32(s);
    }

    // ═══════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════

    /// @dev Mirrors the bot's signer. Must stay byte-identical to
    ///      INTERFACES.md §3 and the worked example in test_goldenVector_emit.
    function _signNav(uint256 newNav, uint256 timestamp, uint256 pk)
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
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, ethSignedHash);
        return abi.encodePacked(r, s, v);
    }
}
