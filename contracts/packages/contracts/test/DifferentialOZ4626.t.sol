// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ERC4626} from "@openzeppelin/contracts/token/ERC20/extensions/ERC4626.sol";
import {ERC20, IERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {MockUSDC} from "./PacificaCarryVault.t.sol";

/// @dev Minimal ref4626 ERC4626 vault built directly on OZ's implementation
///      with no overrides. Used as a "ground truth" against which we verify
///      that our vault's share math (deposit pricing, convertToShares/Assets)
///      matches the standard for the same asset inputs.
contract ReferenceOZ4626 is ERC4626 {
    constructor(IERC20 asset_) ERC20("Reference", "REF") ERC4626(asset_) {}
}

/// @title DifferentialOZ4626Test
/// @notice Side-by-side fuzz: every deposit/convert call on PacificaCarryVault
///         is replicated on a stock OZ ERC4626 ref4626. Any divergence beyond
///         the 70/30 treasury-split effect indicates an accounting bug in our
///         overrides.
///
/// @dev Why this exists. PacificaCarryVault overrides `deposit` to split 30%
///      to the treasury, and `totalAssets()` to add `totalAssetsStored`.
///      Everything else — share minting math, convertToShares/Assets formulas,
///      ERC20 accounting — is inherited from OZ. The differential test pins
///      that inherited behavior so a future refactor that accidentally
///      changes the math fails loud.
///
/// @dev Tolerances. Pacifica's total assets differ from OZ by exactly
///      `totalAssetsStored` (0 in most tests here) plus treasury interest
///      (0 in single-tx scenarios). So for same asset input at t=0, share
///      balances must match exactly.
contract DifferentialOZ4626Test is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault pacifica;
    ReferenceOZ4626 ref4626;

    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice = makeAddr("alice");

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);
        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));
        usdc.mint(address(treasury), 10_000_000e6);

        pacifica = new PacificaCarryVault(
            IERC20(address(usdc)), treasury, operator, guardian, 86400, guardian, 0, 0
        );
        ref4626 = new ReferenceOZ4626(IERC20(address(usdc)));

        usdc.mint(alice, 10_000_000e6);
        vm.prank(alice);
        usdc.approve(address(pacifica), type(uint256).max);
        vm.prank(alice);
        usdc.approve(address(ref4626), type(uint256).max);
    }

    /// @notice Differential fuzz: same deposit → same shares minted on both.
    /// @dev Bounds: 1 wei to 1e12 (1M USDC). Avoids the empty-vault inflation
    ///      attack edge (not a differential concern; both implementations
    ///      exhibit the same OZ default behavior).
    function testFuzz_deposit_sharesMintedMatch(uint256 assets) public {
        assets = bound(assets, 1, 1_000_000e6);

        vm.prank(alice);
        uint256 pacificaShares = pacifica.deposit(assets, alice);
        vm.prank(alice);
        uint256 refShares = ref4626.deposit(assets, alice);

        assertEq(
            pacificaShares,
            refShares,
            "first-deposit share math must match OZ ref4626"
        );
    }

    /// @notice Differential fuzz: convertToAssets and convertToShares match
    ///         OZ ref4626 when no NAV or treasury drift is present.
    function testFuzz_convertRoundTrip_matches(uint256 depositAmt, uint256 queryShares) public {
        depositAmt = bound(depositAmt, 1, 1_000_000e6);

        vm.prank(alice);
        pacifica.deposit(depositAmt, alice);
        vm.prank(alice);
        ref4626.deposit(depositAmt, alice);

        queryShares = bound(queryShares, 0, pacifica.balanceOf(alice));

        // convertToAssets path: idle+treasury in Pacifica == assets in
        // Reference (no stored NAV, no treasury interest yet since no time
        // elapsed). So share→asset conversion must match exactly.
        uint256 pacificaAssets = pacifica.convertToAssets(queryShares);
        uint256 refAssets = ref4626.convertToAssets(queryShares);
        assertEq(pacificaAssets, refAssets, "convertToAssets must match");

        // convertToShares inverse: same inputs → same result.
        uint256 pacificaSharesBack = pacifica.convertToShares(pacificaAssets);
        uint256 refSharesBack = ref4626.convertToShares(refAssets);
        assertEq(pacificaSharesBack, refSharesBack, "convertToShares must match");
    }

    /// @notice totalAssets formula audit: Pacifica = idle + treasury + stored.
    ///         When stored = 0 and no interest accrued, Pacifica.totalAssets
    ///         equals Reference.totalAssets for same deposit size.
    function test_totalAssets_matchesReference_atT0() public {
        uint256 d = 5_000e6;
        vm.prank(alice);
        pacifica.deposit(d, alice);
        vm.prank(alice);
        ref4626.deposit(d, alice);

        assertEq(
            pacifica.totalAssets(),
            ref4626.totalAssets(),
            "totalAssets must match at t=0 with stored=0"
        );
    }

    /// @notice After a NAV report (+X), Pacifica.totalAssets = Reference + X.
    ///         Documents the exact divergence source.
    function test_totalAssets_divergesBy_storedNav() public {
        uint256 d = 5_000e6;
        vm.prank(alice);
        pacifica.deposit(d, alice);
        vm.prank(alice);
        ref4626.deposit(d, alice);

        uint256 ts = block.timestamp + 1;
        vm.warp(ts);
        uint256 storedNav = 100e6;
        bytes32 payloadHash = keccak256(
            abi.encodePacked(
                "PACIFICA_CARRY_VAULT_NAV", address(pacifica), storedNav, ts
            )
        );
        bytes32 ethHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", payloadHash)
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(OPERATOR_PK, ethHash);
        pacifica.reportNAV(storedNav, ts, abi.encodePacked(r, s, v));

        // Divergence = storedNav + tiny treasury drift (1 sec × 5% APY
        // on 30% treasury allocation ≈ 2 wei for 5000 USDC deposit).
        // Tolerance of 10 wei covers the drift without masking real bugs.
        uint256 divergence = pacifica.totalAssets() - ref4626.totalAssets();
        assertApproxEqAbs(
            divergence,
            storedNav,
            10,
            "divergence must equal totalAssetsStored (modulo sub-second treasury drift)"
        );
    }
}
