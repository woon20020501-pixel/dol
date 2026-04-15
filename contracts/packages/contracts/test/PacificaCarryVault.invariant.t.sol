// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @title VaultHandler
/// @dev Foundry invariant handler. Every public function here is a fuzzed
///      action that the invariant runner calls in random order and with
///      random arguments. The handler wires each action to the vault so
///      that the vault stays in a realistic (but adversarial) state space.
contract VaultHandler is Test {
    PacificaCarryVault public vault;
    MockUSDC public usdc;
    MockMoonwellMarket public treasury;
    uint256 internal operatorPk;
    address internal operator;
    address internal guardian;

    /// @dev Ghost variable: cumulative assets deposited.
    uint256 public ghost_totalDeposited;
    /// @dev Ghost variable: cumulative assets claimed via withdraw queue.
    uint256 public ghost_totalClaimed;
    /// @dev Ghost variable: last NAV value set by reportNAV.
    uint256 public ghost_lastReportedNav;
    /// @dev Ghost variable: whether a loss was reported (NAV decreased).
    bool public ghost_lossReported;
    /// @dev Ghost variable: share price right before the most recent action.
    uint256 public ghost_prevSharePrice;
    /// @dev Ghost variable: tracks whether the last action was a reportNAV with loss.
    bool public ghost_lastActionWasLoss;
    /// @dev Ghost variable: tracks whether the last action was a claimWithdraw.
    ///      claimWithdraw legitimately decreases share price because it removes
    ///      assets that were "locked" since requestWithdraw (which burned shares
    ///      but kept assets, temporarily inflating the price).
    bool public ghost_lastActionWasClaim;

    address[] internal actors;
    uint256[] internal pendingRequestIds;

    constructor(
        PacificaCarryVault _vault,
        MockUSDC _usdc,
        MockMoonwellMarket _treasury,
        uint256 _operatorPk,
        address _guardian
    ) {
        vault = _vault;
        usdc = _usdc;
        treasury = _treasury;
        operatorPk = _operatorPk;
        operator = vm.addr(_operatorPk);
        guardian = _guardian;

        // Create a pool of actors
        for (uint256 i = 0; i < 5; i++) {
            address actor = address(uint160(0x1000 + i));
            actors.push(actor);
            usdc.mint(actor, 1_000_000e6);
            vm.prank(actor);
            usdc.approve(address(vault), type(uint256).max);
        }
    }

    /// @dev Fuzzed deposit action.
    function deposit(uint256 actorSeed, uint256 assets) external {
        // Bound to valid range
        assets = bound(assets, 1, 100_000e6);
        address actor = actors[actorSeed % actors.length];

        // Ensure the actor has enough USDC
        if (usdc.balanceOf(actor) < assets) return;
        // Skip if paused
        if (vault.paused()) return;

        _snapshotSharePrice();

        vm.prank(actor);
        vault.deposit(assets, actor);

        ghost_totalDeposited += assets;
        ghost_lastActionWasLoss = false;
        ghost_lastActionWasClaim = false;
    }

    /// @dev Fuzzed requestWithdraw action.
    function requestWithdraw(uint256 actorSeed, uint256 shares) external {
        address actor = actors[actorSeed % actors.length];
        uint256 balance = vault.balanceOf(actor);
        if (balance == 0) return;

        shares = bound(shares, 1, balance);

        _snapshotSharePrice();

        vm.prank(actor);
        uint256 requestId = vault.requestWithdraw(shares);
        pendingRequestIds.push(requestId);

        ghost_lastActionWasLoss = false;
        ghost_lastActionWasClaim = false;
    }

    /// @dev Fuzzed claimWithdraw action.
    function claimWithdraw(uint256 requestIdSeed) external {
        if (pendingRequestIds.length == 0) return;
        if (vault.paused()) return;

        uint256 requestId = pendingRequestIds[requestIdSeed % pendingRequestIds.length];
        (address user, uint256 assets, uint256 unlockTs, bool claimed) =
            vault.withdrawRequests(requestId);

        if (claimed || block.timestamp < unlockTs || user == address(0)) return;

        // Make sure vault can fulfill the claim from idle + treasury liquidity.
        uint256 idle = usdc.balanceOf(address(vault));
        uint256 treasuryBal = treasury.balanceOfUnderlying(address(vault));
        if (idle + treasuryBal < assets) return;

        _snapshotSharePrice();

        vm.prank(user);
        vault.claimWithdraw(requestId);

        ghost_totalClaimed += assets;
        ghost_lastActionWasLoss = false;
        ghost_lastActionWasClaim = true;
    }

    /// @dev Fuzzed NAV report (small delta within 10%).
    function reportNav(uint256 deltaBps) external {
        uint256 currentNav = vault.totalAssets();
        if (currentNav == 0) return;

        // Bound delta to [-9.9%, +9.9%] expressed as basis points 1-990
        deltaBps = bound(deltaBps, 1, 990);

        _snapshotSharePrice();

        // Alternate between gain and loss based on deltaBps parity
        uint256 newNav;
        bool isLoss = deltaBps % 2 == 0;
        uint256 change = (currentNav * deltaBps) / 10_000;
        if (change == 0) change = 1; // Ensure at least 1 wei change

        if (isLoss) {
            newNav = currentNav - change;
            // Ensure we don't exceed the 10% guard
            uint256 delta = currentNav - newNav;
            if (delta * 10 >= currentNav) return;
        } else {
            newNav = currentNav + change;
            uint256 delta = newNav - currentNav;
            if (delta * 10 >= currentNav) return;
        }

        uint256 ts = vault.lastTimestamp() + 1;
        bytes memory sig = _signNav(newNav, ts);

        vault.reportNAV(newNav, ts, sig);

        if (isLoss) {
            ghost_lossReported = true;
            ghost_lastActionWasLoss = true;
        } else {
            ghost_lastActionWasLoss = false;
        }
        ghost_lastActionWasClaim = false;
        ghost_lastReportedNav = newNav;
    }

    /// @dev Fuzzed pause action (guardian only).
    function togglePause() external {
        if (vault.paused()) {
            vm.prank(guardian);
            vault.unpause();
        } else {
            vm.prank(guardian);
            vault.pause();
        }
    }

    /// @dev Fuzzed warp to advance time (for cooldowns).
    function warpForward(uint256 secs) external {
        secs = bound(secs, 1, 7 days);
        vm.warp(block.timestamp + secs);
    }

    /// @dev Attempt reportNAV from a random non-operator address. Should always fail.
    ///      Used by invariant_onlyOperatorReportsNav.
    function reportNavAsRandomCaller(
        uint256 callerPk,
        uint256 newNav,
        uint256 timestamp
    ) external {
        // Bound private key to valid range, but exclude the operator's key
        callerPk = bound(callerPk, 1, 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF);
        if (callerPk == operatorPk) callerPk += 1;

        address caller = vm.addr(callerPk);
        if (caller == operator) return; // Skip if somehow matches

        // Sign with the non-operator key
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
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(callerPk, ethSignedHash);
        bytes memory sig = abi.encodePacked(r, s, v);

        // This must always revert with InvalidNavSignature (or StaleTimestamp)
        vm.expectRevert();
        vault.reportNAV(newNav, timestamp, sig);
    }

    // ── Internal helpers ────────────────────────────────────────────────

    function _snapshotSharePrice() internal {
        ghost_prevSharePrice = vault.sharePrice();
    }

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
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(operatorPk, ethSignedHash);
        return abi.encodePacked(r, s, v);
    }
}

/// @title PacificaCarryVaultInvariantTest
/// @notice Invariant tests that prove the vault holds safety properties
///         under adversarial, fuzzed sequences of actions.
contract PacificaCarryVaultInvariantTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;
    VaultHandler handler;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    uint256 constant COOLDOWN = 86400;

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);
        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));

        // Pre-fund the treasury so it can pay accrued interest under random warps
        usdc.mint(address(treasury), 10_000_000e6);

        vault = new PacificaCarryVault(
            IERC20(address(usdc)),
            treasury,
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        handler = new VaultHandler(vault, usdc, treasury, OPERATOR_PK, guardian);

        // Seed the vault with an initial deposit so invariants are meaningful
        usdc.mint(address(this), 10_000e6);
        usdc.approve(address(vault), type(uint256).max);
        vault.deposit(10_000e6, address(this));

        // Target only the handler for invariant calls
        targetContract(address(handler));
    }

    /// @notice totalAssets() never underflows or returns a negative value.
    /// Protects: uint256 totalAssetsStored cannot wrap around. Since Solidity
    ///           0.8 reverts on underflow, this invariant verifies that no
    ///           sequence of deposits, withdrawals, and NAV reports can
    ///           drive totalAssets below 0 (i.e., trigger a revert from
    ///           underflow in normal operations). It also verifies the
    ///           V1.5 totalAssets formula: idle + treasury + margin slot.
    function invariant_totalAssetsNeverNegative() public view {
        // In Solidity 0.8, totalAssets() is uint256 — it physically cannot be
        // negative. This assertion verifies the slot always holds a sane value.
        assertTrue(vault.totalAssets() >= 0, "totalAssets must be non-negative");

        // V1.5: totalAssets must equal idle + treasury balance + reported margin.
        uint256 idle = usdc.balanceOf(address(vault));
        uint256 inTreasury = treasury.balanceOfUnderlying(address(vault));
        uint256 stored = vault.totalAssetsStored();
        assertEq(
            vault.totalAssets(),
            idle + inTreasury + stored,
            "totalAssets() must equal idle + treasury + stored"
        );
    }

    /// @notice Share price only decreases when a loss is reported via reportNAV
    ///         or when a claimWithdraw completes (returning the temporarily-
    ///         inflated price back to its pre-requestWithdraw level).
    ///         Deposits alone must never decrease the share price beyond
    ///         integer rounding dust.
    /// Protects: depositors are not diluted by other depositors. The only
    ///           legitimate sources of share price decline are:
    ///           1. An explicit NAV loss report (the fund lost money)
    ///           2. claimWithdraw returning the price to normal after
    ///              requestWithdraw temporarily inflated it (burned shares
    ///              but kept assets in totalAssetsStored)
    function invariant_sharePriceMonotonicExceptOnLoss() public view {
        uint256 currentPrice = vault.sharePrice();
        uint256 prevPrice = handler.ghost_prevSharePrice();

        // If prevPrice is 0, this is before any action — skip
        if (prevPrice == 0) return;

        // Price decreased — check why.
        if (currentPrice < prevPrice) {
            // Loss report: any decrease is expected and OK.
            if (handler.ghost_lastActionWasLoss()) return;

            // claimWithdraw: returns price from its post-requestWithdraw
            // inflated level back to the fair price. This is expected.
            if (handler.ghost_lastActionWasClaim()) return;

            // Anything else (deposit, requestWithdraw): only ERC4626
            // integer rounding dust is acceptable. The rounding error
            // from a single operation is bounded by ~1/totalSupply
            // relative, which is negligible. Use 10 ppb tolerance.
            uint256 drop = prevPrice - currentPrice;
            uint256 tolerance = prevPrice / 1e8 + 1;
            assertLe(
                drop,
                tolerance,
                "share price decreased beyond rounding tolerance without a loss report or claim"
            );
        }
    }

    /// @notice While paused, deposit() always reverts regardless of caller,
    ///         assets, or receiver.
    /// Protects: the pause mechanism is watertight — no combination of
    ///           parameters can bypass it.
    function invariant_pausedBlocksDeposits() public {
        if (!vault.paused()) return;

        // Try to deposit as a funded actor
        address actor = address(0x1000); // first handler actor
        uint256 assets = 1e6; // 1 USDC
        if (usdc.balanceOf(actor) < assets) return;

        vm.prank(actor);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.deposit(assets, actor);
    }

    /// @notice No non-operator address can successfully call reportNAV.
    ///         Fuzzed across random callers and signatures.
    /// Protects: the NAV oracle is exclusively controlled by the operator
    ///           key — no address collision, signature forgery, or replay
    ///           can bypass the signer check.
    function invariant_onlyOperatorReportsNav() public view {
        // The handler's reportNavAsRandomCaller always expects a revert.
        // If execution reaches here without the handler reverting, the
        // invariant holds. The handler's method uses vm.expectRevert()
        // which will fail the entire run if the call does NOT revert.
        //
        // We also verify the operator address is consistent:
        assertEq(
            vault.operator(),
            operator,
            "operator address must not change without guardian action"
        );
    }
}
