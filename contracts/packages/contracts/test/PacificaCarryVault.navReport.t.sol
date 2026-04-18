// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title PacificaCarryVaultNavReportTest
/// @notice End-to-end integration tests that exercise the NAV-report signing
///         flow exactly the way the off-chain operator bot will. Each test
///         constructs the signing payload from raw fields, hashes with the
///         EIP-191 prefix, signs with a known operator private key, and
///         submits via `reportNAV`. These tests serve as the reference the
///         bot mirrors — if any test in this file breaks, the bot's signer
///         is also broken.
contract PacificaCarryVaultNavReportTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;

    // Deterministic operator key so the off-chain bot can reproduce signatures locally.
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
            guardian,
            0,
            0
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
    /// Protects: the full signing flow the off-chain bot will run — payload encoding,
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
    // GOLDEN VECTOR — REFERENCE BYTES FOR THE OFF-CHAIN SIGNER TO MIRROR
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
    // C4 RATE-LIMIT TESTS (2026-04-17)
    //
    // Parameter basis: MakerDAO OSM `hop = 3600s`, Chainlink BTC/USD
    // heartbeat 3600s, Lido OracleReportSanityChecker daily cap pattern.
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Rate-limited vault rejects a second report within the
    ///         `minReportInterval` window.
    /// Protects: C4 — post-compromise compounded drift via chained
    ///           reports is bounded by the interval.
    function test_c4_minInterval_rejectsFastFollowup() public {
        // Fresh vault with production-grade rate limit (3600s = Chainlink/OSM)
        PacificaCarryVault limitedVault = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian,
            3600, // minReportInterval
            0 // no daily cap (isolate the interval check)
        );
        usdc.mint(alice, SEED_DEPOSIT);
        vm.prank(alice);
        usdc.approve(address(limitedVault), type(uint256).max);
        vm.prank(alice);
        limitedVault.deposit(SEED_DEPOSIT, alice);

        // Use absolute timestamps anchored to a fixed base so the sequence
        // is unambiguous regardless of Foundry's block-timestamp default.
        uint256 BASE = 1_000_000_000;

        // First report at BASE establishes baseline
        vm.warp(BASE);
        limitedVault.reportNAV(100e6, BASE, _signNavFor(limitedVault, 100e6, BASE));
        assertEq(limitedVault.lastTimestamp(), BASE, "first report recorded");

        // Second report 1 second later (inside 3600s interval) → revert
        vm.warp(BASE + 1);
        bytes memory sig2 = _signNavFor(limitedVault, 101e6, BASE + 1);
        vm.expectRevert(PacificaCarryVault.NavReportTooFrequent.selector);
        limitedVault.reportNAV(101e6, BASE + 1, sig2);

        // At exactly BASE + 3599 → still revert (strict < boundary)
        vm.warp(BASE + 3599);
        bytes memory sig3 = _signNavFor(limitedVault, 101e6, BASE + 3599);
        vm.expectRevert(PacificaCarryVault.NavReportTooFrequent.selector);
        limitedVault.reportNAV(101e6, BASE + 3599, sig3);

        // At exactly BASE + 3600 → accepted (boundary)
        vm.warp(BASE + 3600);
        limitedVault.reportNAV(
            101e6, BASE + 3600, _signNavFor(limitedVault, 101e6, BASE + 3600)
        );
        assertEq(limitedVault.totalAssetsStored(), 101e6, "C4: boundary report accepted");
    }

    /// @notice Daily cumulative delta cap rejects reports that would push
    ///         the UTC-day sum over the threshold.
    /// Protects: C4 — cumulative drift over many reports capped in bps/day.
    function test_c4_dailyCap_rejectsExcess() public {
        // Production-style limit: 100 bps/day cap (Lido-style), no min interval
        // so we can test the daily cap in isolation
        PacificaCarryVault cappedVault = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian,
            0,   // no min interval (isolate daily cap)
            100  // 100 bps/day
        );
        usdc.mint(alice, SEED_DEPOSIT);
        vm.prank(alice);
        usdc.approve(address(cappedVault), type(uint256).max);
        vm.prank(alice);
        cappedVault.deposit(SEED_DEPOSIT, alice);

        // First report: baseline 100_000e6 (1% over daily cap at 100 USDC for Alice's
        // deposit, but baseline is 100k pure oracle slot — daily cap applies to
        // subsequent deltas against this baseline).
        uint256 ts0 = block.timestamp + 1;
        vm.warp(ts0);
        cappedVault.reportNAV(100_000e6, ts0, _signNavFor(cappedVault, 100_000e6, ts0));

        // Second report +50 bps (0.5%) → under cap → accepted
        uint256 nav1 = 100_500e6; // +0.5%
        uint256 ts1 = ts0 + 1;
        vm.warp(ts1);
        cappedVault.reportNAV(nav1, ts1, _signNavFor(cappedVault, nav1, ts1));
        assertEq(cappedVault.totalAssetsStored(), nav1, "C4.1: 0.5% accepted");

        // Third report +40 bps more (cumulative 90 bps same day) → still under
        uint256 nav2 = 100_900e6;
        uint256 ts2 = ts1 + 1;
        vm.warp(ts2);
        cappedVault.reportNAV(nav2, ts2, _signNavFor(cappedVault, nav2, ts2));
        assertEq(cappedVault.totalAssetsStored(), nav2, "C4.2: cumulative 0.9% accepted");

        // Fourth report +20 bps more (would reach 110 bps cumulative) → revert
        uint256 nav3 = 101_100e6;
        uint256 ts3 = ts2 + 1;
        vm.warp(ts3);
        bytes memory sig3 = _signNavFor(cappedVault, nav3, ts3);
        vm.expectRevert(PacificaCarryVault.NavDailyDeltaExceeded.selector);
        cappedVault.reportNAV(nav3, ts3, sig3);

        // Next-day bucket resets the counter
        uint256 ts4 = ts3 + 1 days;
        vm.warp(ts4);
        cappedVault.reportNAV(nav3, ts4, _signNavFor(cappedVault, nav3, ts4));
        assertEq(cappedVault.totalAssetsStored(), nav3, "C4.3: next-day resets counter");
    }

    /// @notice Daily-cap strict `>` boundary check on the CURRENT `lastNav`.
    /// @dev Cap is expressed as `newDaySum * 10_000 > lastNav * maxBps`.
    ///      With `lastNav = 100_000e6` (baseline) and `maxBps = 100` (1%),
    ///      `newDaySum > 100_000e6 * 100 / 10_000 = 1_000e6` fires the revert.
    ///      Since `lastNav` refreshes after each accepted report, this test
    ///      fixes the baseline snapshot and verifies:
    ///        (a) a single report that adds exactly 1_000e6 delta → accepted
    ///            (sum = 1_000e6, not > cap 1_000e6 against baseline)
    ///        (b) a subsequent report that would push sum beyond the NEW
    ///            lastNav's cap → reverts
    ///      Accepts the documented "drifting denominator" semantics; a
    ///      tighter cap would require storing the snapshot baseline per day.
    function test_c4_dailyCap_exactBoundary() public {
        PacificaCarryVault v = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian,
            0, 100 // 100 bps daily cap
        );
        usdc.mint(alice, SEED_DEPOSIT);
        vm.prank(alice);
        usdc.approve(address(v), type(uint256).max);
        vm.prank(alice);
        v.deposit(SEED_DEPOSIT, alice);

        uint256 ts0 = 1_000_000_000;
        vm.warp(ts0);
        v.reportNAV(100_000e6, ts0, _signNavFor(v, 100_000e6, ts0));

        // Exactly +1_000e6 delta (100 bps of the 100_000e6 baseline)
        uint256 ts1 = ts0 + 1;
        vm.warp(ts1);
        uint256 nav1 = 101_000e6;
        v.reportNAV(nav1, ts1, _signNavFor(v, nav1, ts1));
        assertEq(v.totalAssetsStored(), nav1, "C4.boundary.a: exactly 100 bps accepted");

        // Second report with large enough delta to push cumulative beyond
        // the NEW lastNav's 1% bound. lastNav is now 101_000e6; daily cap
        // absolute = 101_000e6 * 100 / 10_000 = 1_010e6. Sum already 1_000e6.
        // A delta > 10e6 would push sum > 1_010e6 → revert.
        uint256 ts2 = ts1 + 1;
        vm.warp(ts2);
        uint256 nav2 = 101_000e6 + 11e6; // +11e6 delta pushes sum to 1_011e6
        bytes memory sig2 = _signNavFor(v, nav2, ts2);
        vm.expectRevert(PacificaCarryVault.NavDailyDeltaExceeded.selector);
        v.reportNAV(nav2, ts2, sig2);

        // But exactly at the boundary (+10e6 → sum = 1_010e6) is accepted
        uint256 nav3 = 101_000e6 + 10e6;
        v.reportNAV(nav3, ts2, _signNavFor(v, nav3, ts2));
        assertEq(
            v.totalAssetsStored(),
            nav3,
            "C4.boundary.b: exactly-at-cap accepted"
        );
    }

    /// @notice View: dailyDeltaConsumed reports the accumulator.
    function test_c4_dailyDeltaConsumed_view() public {
        PacificaCarryVault cappedVault = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, COOLDOWN, guardian,
            0, 100
        );
        uint256 ts0 = block.timestamp + 1;
        vm.warp(ts0);
        cappedVault.reportNAV(100_000e6, ts0, _signNavFor(cappedVault, 100_000e6, ts0));
        uint256 day = ts0 / 1 days;
        assertEq(cappedVault.dailyDeltaConsumed(day), 0, "C4.4: baseline report adds 0");

        uint256 ts1 = ts0 + 1;
        vm.warp(ts1);
        cappedVault.reportNAV(100_500e6, ts1, _signNavFor(cappedVault, 100_500e6, ts1));
        uint256 day1 = ts1 / 1 days;
        assertEq(
            cappedVault.dailyDeltaConsumed(day1),
            500e6,
            "C4.5: 0.5% delta tracked"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════

    /// @dev Sign for an arbitrary vault address (for multi-vault tests).
    function _signNavFor(PacificaCarryVault v, uint256 newNav, uint256 timestamp)
        internal
        pure
        returns (bytes memory)
    {
        bytes32 payloadHash = keccak256(
            abi.encodePacked("PACIFICA_CARRY_VAULT_NAV", address(v), newNav, timestamp)
        );
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", payloadHash)
        );
        (uint8 v_, bytes32 r, bytes32 s) = vm.sign(OPERATOR_PK, ethSignedHash);
        return abi.encodePacked(r, s, v_);
    }

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
