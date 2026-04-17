// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20, IERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {PacificaCarryVault} from "./PacificaCarryVault.sol";
import {IPBondJunior} from "./IPBondJunior.sol";

/// @title Dol
/// @notice ERC-20 token representing the Dol interest-bearing product — the
///         user-facing single-tier structured deposit on top of the
///         PacificaCarryVault. Target yield: 7.50% APY. The legacy two-tier
///         (Senior/Junior) architecture is retained in the code path for
///         future reactivation, but the Junior tranche is DEACTIVATED for
///         Phase 1 launch: `juniorContract` is left unset at deploy time,
///         so `distributeYield()` reverts with `JuniorNotSet` and no yield
///         waterfall runs. This makes Dol a pure Senior-only claim on the
///         vault.
/// @dev Architecture:
///      - deposit(): USDC -> vault.deposit -> mint DOL 1:1
///      - redeem(): burn DOL -> vault.requestWithdraw -> cooldown queue (scheduled)
///      - instantRedeem(): burn DOL -> vault.instantRedeem -> USDC in one tx
///      - claimRedeem(): vault.claimWithdraw -> USDC to user (scheduled payout)
///      - distributeYield(): DISABLED at runtime via unset juniorContract.
///        Left in the source so a future Dol+Junior relaunch can flip it
///        back on by deploying a fresh Dol that calls setJuniorContract.
contract Dol is ERC20, ReentrancyGuard {
    using SafeERC20 for IERC20;

    // ── Errors ─────────────────────────────────────────────────────────
    error ZeroAmount();
    error OnlyGuardian();
    error JuniorAlreadySet();
    error JuniorNotSet();
    error NotRedeemOwner();
    error AlreadyClaimed();
    error InsufficientBalance();

    // ── Events ─────────────────────────────────────────────────────────
    /// @notice Emitted when a user deposits USDC and receives DOL tokens.
    event Deposited(address indexed user, uint256 usdcAmount, uint256 dolMinted);
    /// @notice Emitted when a user initiates a scheduled redemption (starts cooldown).
    event RedeemRequested(address indexed user, uint256 dolBurned, uint256 redeemId);
    /// @notice Emitted when a user claims USDC after cooldown.
    event RedeemClaimed(address indexed user, uint256 redeemId, uint256 usdcReturned);
    /// @notice Emitted after yield distribution between Senior (Dol) and Junior.
    ///         Unused at runtime in Phase 1 (Junior deactivated).
    event YieldDistributed(uint256 seniorYield, uint256 juniorYield);
    /// @notice Emitted when a user instantly redeems DOL for USDC.
    event InstantRedeemed(address indexed user, uint256 dolBurned, uint256 usdcOut);

    // ── Immutables ─────────────────────────────────────────────────────
    /// @notice The underlying PacificaCarryVault that holds the strategy assets.
    PacificaCarryVault public immutable vault;
    /// @notice The USDC token (underlying asset).
    IERC20 public immutable usdc;

    // ── State ──────────────────────────────────────────────────────────
    /// @notice Guardian address — can set Junior contract and trigger yield distribution.
    address public immutable guardian;
    /// @notice Address of the paired Junior tranche contract.
    /// @dev Phase 1: intentionally left at address(0). `distributeYield` reverts
    ///      with `JuniorNotSet`, making Dol behave as a Senior-only product.
    address public juniorContract;
    /// @notice Target annual yield for Dol (Senior tranche), in basis points.
    uint16 public constant SENIOR_TARGET_APY_BPS = 750; // 7.50%
    /// @notice Basis-point denominator (10000 = 100%).
    uint256 public constant BPS_DENOMINATOR = 10_000;
    /// @notice Cumulative USDC principal deposited (decremented on redeem).
    uint256 public totalDeposited;
    /// @notice Timestamp of the last yield distribution.
    uint256 public lastDistributionTimestamp;

    /// @notice Tracks a pending scheduled redemption through the vault's withdraw queue.
    struct RedeemRequest {
        address user;
        uint256 vaultRequestId;
        bool claimed;
    }
    /// @notice Mapping from Dol-side redeemId to the request details.
    mapping(uint256 => RedeemRequest) public redeemRequests;
    /// @notice Next redeem request ID (monotonically increasing).
    uint256 public nextRedeemId;

    // ── Constructor ────────────────────────────────────────────────────

    /// @notice Deploy the Dol token.
    /// @param _vault Address of the PacificaCarryVault
    /// @param _usdc Address of the USDC token
    /// @param _guardian Address that can set Junior and trigger distribution
    constructor(PacificaCarryVault _vault, IERC20 _usdc, address _guardian) ERC20("Dol", "DOL") {
        vault = _vault;
        usdc = _usdc;
        guardian = _guardian;
        lastDistributionTimestamp = block.timestamp;
    }

    /// @notice DOL uses 6 decimals to match USDC.
    function decimals() public pure override returns (uint8) {
        return 6;
    }

    // ── Deposit ────────────────────────────────────────────────────────

    /// @notice Deposit USDC and receive DOL 1:1.
    /// @dev Transfers USDC from the caller, deposits into the vault (which
    ///      splits 70/30 between idle and treasury), and mints DOL tokens.
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

    // ── Redeem (two-step scheduled path via vault queue) ──────────────

    /// @notice Request a scheduled redemption. Burns DOL and starts the
    ///         vault's withdraw cooldown. Returns a redeemId for claiming later.
    /// @param dolAmount Amount of DOL tokens to burn
    /// @return redeemId The Dol-side request ID
    function redeem(uint256 dolAmount) external nonReentrant returns (uint256 redeemId) {
        if (dolAmount == 0) revert ZeroAmount();
        if (balanceOf(msg.sender) < dolAmount) revert InsufficientBalance();

        uint256 supply = totalSupply();
        uint256 vaultShareBal = IERC20(address(vault)).balanceOf(address(this));
        uint256 vaultShares = (dolAmount * vaultShareBal) / supply;
        uint256 principalReduced = (dolAmount * totalDeposited) / supply;

        _burn(msg.sender, dolAmount);
        totalDeposited -= principalReduced;

        uint256 vaultRequestId = vault.requestWithdraw(vaultShares);
        redeemId = nextRedeemId++;
        redeemRequests[redeemId] = RedeemRequest({user: msg.sender, vaultRequestId: vaultRequestId, claimed: false});
        emit RedeemRequested(msg.sender, dolAmount, redeemId);
    }

    /// @notice Instant-redeem DOL for USDC in a single transaction.
    /// @dev Burns DOL, converts to vault shares pro-rata over Dol's
    ///      share balance, then calls `vault.instantRedeem()` which pays
    ///      the 0.05% fee to the vault-level feeRecipient and returns net
    ///      USDC. Forwards the net USDC to the caller. Fails if the vault's
    ///      idle USDC buffer is insufficient — frontend should catch and
    ///      route the user to the Scheduled path.
    /// @param dolAmount Amount of DOL tokens to burn
    /// @return usdcOut Net USDC transferred to the caller (after vault fee)
    function instantRedeem(uint256 dolAmount) external nonReentrant returns (uint256 usdcOut) {
        if (dolAmount == 0) revert ZeroAmount();
        if (balanceOf(msg.sender) < dolAmount) revert InsufficientBalance();

        uint256 supply = totalSupply();
        uint256 vaultShareBal = IERC20(address(vault)).balanceOf(address(this));
        uint256 vaultShares = (dolAmount * vaultShareBal) / supply;
        uint256 principalReduced = (dolAmount * totalDeposited) / supply;

        _burn(msg.sender, dolAmount);
        totalDeposited -= principalReduced;

        usdcOut = vault.instantRedeem(vaultShares);
        usdc.safeTransfer(msg.sender, usdcOut);

        emit InstantRedeemed(msg.sender, dolAmount, usdcOut);
    }

    /// @notice Claim USDC after the vault's cooldown has elapsed.
    /// @param redeemId The Dol-side request ID from redeem()
    function claimRedeem(uint256 redeemId) external nonReentrant {
        RedeemRequest storage req = redeemRequests[redeemId];
        if (req.user != msg.sender) revert NotRedeemOwner();
        if (req.claimed) revert AlreadyClaimed();
        req.claimed = true;
        uint256 assets = vault.claimWithdraw(req.vaultRequestId);
        usdc.safeTransfer(msg.sender, assets);
        emit RedeemClaimed(msg.sender, redeemId, assets);
    }

    // ── Yield Distribution (DISABLED at runtime in Phase 1) ───────────

    /// @notice Distribute yield between Senior (Dol) and Junior tranches.
    /// @dev Phase 1: reverts immediately with `JuniorNotSet` because
    ///      `juniorContract == address(0)`. Retained for a future Dol+Junior
    ///      relaunch path.
    function distributeYield() external {
        if (juniorContract == address(0)) revert JuniorNotSet();

        uint256 elapsed = block.timestamp - lastDistributionTimestamp;
        lastDistributionTimestamp = block.timestamp;
        // slither-disable-next-line incorrect-equality
        if (elapsed == 0) return;

        uint256 seniorVaultShares = IERC20(address(vault)).balanceOf(address(this));
        // slither-disable-next-line incorrect-equality
        if (seniorVaultShares == 0) return;
        uint256 seniorValue = vault.convertToAssets(seniorVaultShares);

        uint256 seniorYield = (totalDeposited * SENIOR_TARGET_APY_BPS * elapsed) / (BPS_DENOMINATOR * 365 days);
        uint256 seniorTarget = totalDeposited + seniorYield;

        if (seniorValue > seniorTarget) {
            uint256 excessValue = seniorValue - seniorTarget;
            uint256 excessShares = vault.convertToShares(excessValue);
            if (excessShares > 0 && excessShares <= seniorVaultShares) {
                IERC20(address(vault)).safeTransfer(juniorContract, excessShares);
            }
            emit YieldDistributed(seniorYield, excessValue);
        } else if (seniorValue < totalDeposited) {
            uint256 deficit = totalDeposited - seniorValue;
            uint256 juniorVaultShares = IERC20(address(vault)).balanceOf(juniorContract);
            if (juniorVaultShares > 0) {
                uint256 juniorValue = vault.convertToAssets(juniorVaultShares);
                uint256 coverValue = deficit < juniorValue ? deficit : juniorValue;
                uint256 coverShares = vault.convertToShares(coverValue);
                // Defensive cap: coverShares ≤ juniorVaultShares holds by
                // construction of convertToShares/convertToAssets (both floor-
                // round), so this branch is mathematically unreachable in normal
                // operation. Retained as belt-and-suspenders against future
                // rounding-convention changes in the OZ ERC4626 base.
                // slither-disable-next-line dead-code
                if (coverShares > juniorVaultShares) coverShares = juniorVaultShares;
                if (coverShares > 0) {
                    IPBondJunior(juniorContract).absorbLoss(coverShares);
                }
            }
            emit YieldDistributed(0, 0);
        } else {
            emit YieldDistributed(seniorValue - totalDeposited, 0);
        }
    }

    // ── Admin ──────────────────────────────────────────────────────────

    /// @notice Set the paired Junior tranche contract. One-time only.
    /// @dev Phase 1: this is intentionally NOT called during deployment —
    ///      Dol launches Junior-less. If a future phase reactivates Junior,
    ///      deploy a fresh Dol and call this at that time.
    /// @param _junior Address of the Junior tranche contract
    function setJuniorContract(address _junior) external {
        if (msg.sender != guardian) revert OnlyGuardian();
        if (juniorContract != address(0)) revert JuniorAlreadySet();
        juniorContract = _junior;
    }

    // ── View ───────────────────────────────────────────────────────────

    /// @notice Returns the current value of 1 DOL in USDC (6 decimals).
    /// @return price The price per DOL, scaled to 1e6
    function pricePerShare() external view returns (uint256) {
        uint256 supply = totalSupply();
        if (supply == 0) return 1e6;
        uint256 value = vault.convertToAssets(IERC20(address(vault)).balanceOf(address(this)));
        return (value * 1e6) / supply;
    }
}
