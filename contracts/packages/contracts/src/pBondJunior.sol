// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20, IERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {PacificaCarryVault} from "./PacificaCarryVault.sol";
import {IPBondJunior} from "./IPBondJunior.sol";

/// @title pBondJunior
/// @notice ERC-20 wrapper representing the Junior tranche of the pBond
///         structured product. Junior depositors receive excess yield beyond
///         Senior's 7.50% APY target. In loss scenarios, Junior absorbs
///         losses first — acting as the first-loss buffer for Senior holders.
/// @dev Architecture:
///      - deposit(): USDC -> vault.deposit -> mint pBJ 1:1
///      - redeem(): burn pBJ -> vault.requestWithdraw -> cooldown queue
///      - claimRedeem(): vault.claimWithdraw -> USDC to user
///      - absorbLoss(): called by Senior to transfer vault shares from
///        Junior to Senior during loss events.
contract pBondJunior is ERC20, ReentrancyGuard, IPBondJunior {
    using SafeERC20 for IERC20;

    // ── Errors ─────────────────────────────────────────────────────────
    error ZeroAmount();
    error OnlySenior();
    error NotRedeemOwner();
    error AlreadyClaimed();
    error InsufficientBalance();
    error SeniorAlreadySet();
    /// @notice Thrown when a non-deployer attempts to call setSeniorContract.
    /// @dev C1 fix (2026-04-17): v1 had a permissionless setter. Anyone could
    ///      front-run the deployer's setup tx and plant a malicious senior,
    ///      then drain Junior's vault shares via absorbLoss(). The deployer
    ///      is now locked in at construction and is the only address allowed
    ///      to perform the one-time senior link.
    error NotDeployer();

    // ── Events ─────────────────────────────────────────────────────────
    /// @notice Emitted when a user deposits USDC and receives pBJ tokens.
    event Deposited(address indexed user, uint256 usdcAmount, uint256 pbjMinted);
    /// @notice Emitted when a user initiates a redemption (starts cooldown).
    event RedeemRequested(address indexed user, uint256 pbjBurned, uint256 redeemId);
    /// @notice Emitted when a user claims USDC after cooldown.
    event RedeemClaimed(address indexed user, uint256 redeemId, uint256 usdcReturned);
    /// @notice Emitted when Junior absorbs a loss by transferring vault shares to Senior.
    event LossAbsorbed(uint256 vaultSharesTransferred);

    // ── Immutables ─────────────────────────────────────────────────────
    /// @notice The underlying PacificaCarryVault that holds the strategy assets.
    PacificaCarryVault public immutable vault;
    /// @notice The USDC token (underlying asset).
    IERC20 public immutable usdc;

    // ── State ──────────────────────────────────────────────────────────
    /// @notice Address of the paired Senior tranche contract.
    address public seniorContract;
    /// @notice Cumulative USDC principal deposited (decremented on redeem).
    uint256 public totalDeposited;

    /// @notice Deployer address — the only account permitted to call
    ///         `setSeniorContract` exactly once. Recorded at construction
    ///         and never mutated thereafter.
    /// @dev C1 fix (2026-04-17). Rationale in NotDeployer error NatSpec.
    address private immutable _deployer;

    /// @notice Tracks a pending redemption through the vault's withdraw queue.
    struct RedeemRequest {
        address user;
        uint256 vaultRequestId;
        bool claimed;
    }
    /// @notice Mapping from Junior redeemId to the request details.
    mapping(uint256 => RedeemRequest) public redeemRequests;
    /// @notice Next redeem request ID (monotonically increasing).
    uint256 public nextRedeemId;

    // ── Constructor ────────────────────────────────────────────────────

    /// @notice Deploy the Junior tranche wrapper.
    /// @param _vault Address of the PacificaCarryVault
    /// @param _usdc Address of the USDC token
    constructor(PacificaCarryVault _vault, IERC20 _usdc) ERC20("pBond Junior", "pBJ") {
        vault = _vault;
        usdc = _usdc;
        _deployer = msg.sender;
    }

    /// @notice pBJ uses 6 decimals to match USDC.
    function decimals() public pure override returns (uint8) {
        return 6;
    }

    // ── Deposit ────────────────────────────────────────────────────────

    /// @notice Deposit USDC into the Junior tranche. Mints pBJ 1:1.
    /// @param usdcAmount Amount of USDC to deposit (6 decimals)
    function deposit(uint256 usdcAmount) external nonReentrant {
        if (usdcAmount == 0) revert ZeroAmount();
        usdc.safeTransferFrom(msg.sender, address(this), usdcAmount);
        usdc.forceApprove(address(vault), usdcAmount);
        // slither-disable-next-line unused-return
        vault.deposit(usdcAmount, address(this));
        _mint(msg.sender, usdcAmount);
        totalDeposited += usdcAmount;
        emit Deposited(msg.sender, usdcAmount, usdcAmount);
    }

    // ── Redeem (two-step via vault queue) ──────────────────────────────

    /// @notice Request a redemption. Burns pBJ and starts the vault's
    ///         withdraw cooldown.
    /// @param pbjAmount Amount of pBJ tokens to burn
    /// @return redeemId The Junior-side request ID
    function redeem(uint256 pbjAmount) external nonReentrant returns (uint256 redeemId) {
        if (pbjAmount == 0) revert ZeroAmount();
        if (balanceOf(msg.sender) < pbjAmount) revert InsufficientBalance();

        uint256 supply = totalSupply();
        uint256 vaultShareBal = IERC20(address(vault)).balanceOf(address(this));
        uint256 vaultShares = (pbjAmount * vaultShareBal) / supply;
        uint256 principalReduced = (pbjAmount * totalDeposited) / supply;

        _burn(msg.sender, pbjAmount);
        totalDeposited -= principalReduced;

        uint256 vaultRequestId = vault.requestWithdraw(vaultShares);
        redeemId = nextRedeemId++;
        redeemRequests[redeemId] = RedeemRequest({user: msg.sender, vaultRequestId: vaultRequestId, claimed: false});
        emit RedeemRequested(msg.sender, pbjAmount, redeemId);
    }

    /// @notice Claim USDC after the vault's cooldown has elapsed.
    /// @param redeemId The Junior-side request ID from redeem()
    function claimRedeem(uint256 redeemId) external nonReentrant {
        RedeemRequest storage req = redeemRequests[redeemId];
        if (req.user != msg.sender) revert NotRedeemOwner();
        if (req.claimed) revert AlreadyClaimed();
        req.claimed = true;
        uint256 assets = vault.claimWithdraw(req.vaultRequestId);
        usdc.safeTransfer(msg.sender, assets);
        emit RedeemClaimed(msg.sender, redeemId, assets);
    }

    // ── Senior coordination ────────────────────────────────────────────

    /// @notice Transfer vault shares to Senior to cover a loss.
    /// @dev Called by Senior's distributeYield(). Only the Senior contract
    ///      can invoke this. The vault shares are transferred directly
    ///      from Junior's balance to Senior.
    /// @param vaultShares Number of vault shares to transfer to Senior
    function absorbLoss(uint256 vaultShares) external override {
        if (msg.sender != seniorContract) revert OnlySenior();
        IERC20(address(vault)).safeTransfer(seniorContract, vaultShares);
        emit LossAbsorbed(vaultShares);
    }

    /// @notice Set the paired Senior tranche contract. One-time only.
    /// @dev Called during deployment setup. Only the deployer (address that
    ///      executed the constructor) may call this, and only once before
    ///      `seniorContract` is set. Subsequent calls revert with
    ///      `SeniorAlreadySet` even for the deployer — the link is effectively
    ///      immutable after the single initial call.
    /// @custom:security C1 fix (2026-04-17). v1 had a permissionless setter
    ///      that any actor could front-run to plant a malicious senior and
    ///      drain Junior via `absorbLoss`. Deployer-gating blocks the
    ///      front-run vector while preserving the two-step deployment flow
    ///      (Senior and Junior must reference each other post-construction).
    /// @param _senior Address of the senior tranche contract (Dol in Phase 1)
    function setSeniorContract(address _senior) external {
        if (msg.sender != _deployer) revert NotDeployer();
        if (seniorContract != address(0)) revert SeniorAlreadySet();
        seniorContract = _senior;
    }

    // ── View ───────────────────────────────────────────────────────────

    /// @notice Returns the current value of 1 pBJ in USDC (6 decimals).
    /// @return price The price per pBJ share, scaled to 1e6
    function pricePerShare() external view returns (uint256) {
        uint256 supply = totalSupply();
        if (supply == 0) return 1e6;
        uint256 value = vault.convertToAssets(IERC20(address(vault)).balanceOf(address(this)));
        return (value * 1e6) / supply;
    }
}
