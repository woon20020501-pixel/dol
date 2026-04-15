// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC4626} from "@openzeppelin/contracts/token/ERC20/extensions/ERC4626.sol";
import {ERC20, IERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {IMoonwellMarket} from "./IMoonwellMarket.sol";

/// @title PacificaCarryVault
/// @author Pacifica Finance (hackathon build — not audited)
/// @notice ERC-4626 style vault for the Pacifica FX Carry strategy.
///         Accepts USDC deposits, issues shares, and uses a signed NAV oracle
///         to mark-to-market the vault's off-chain positions. Withdrawals go
///         through a two-step queue with a configurable cooldown period.
/// @dev Architecture (V1.5 — two-tier yield):
///      - Deposit: each deposit is split 70/30 between the perp margin staging
///        bucket (idle USDC waiting for the off-chain executor) and a permissionless
///        lending market (treasury) for base yield. Standard ERC-4626 deposit with
///        pause check and reentrancy guard.
///      - Withdraw: `requestWithdraw` burns shares immediately and locks assets
///        for `cooldownSeconds`. `claimWithdraw` pays out after the cooldown,
///        pulling from idle USDC first then redeeming from the treasury vault
///        if necessary. Standard ERC-4626 `withdraw`, `redeem`, and `mint` are
///        disabled.
///      - NAV: the operator submits signed NAV reports via `reportNAV`, which
///        update the off-chain perp margin slot (`totalAssetsStored`). A strict
///        10% sanity guard prevents catastrophic oracle manipulation. Share
///        price is derived from total assets across all three buckets.
///      - totalAssets() = idle USDC + treasury underlying balance + reported margin
///      - Access: OPERATOR_ROLE (bot) can sign NAV reports. GUARDIAN_ROLE (governance)
///        can pause/unpause and rotate keys. No DEFAULT_ADMIN_ROLE is granted.
///      - Safety: ReentrancyGuard on all state-changing external functions,
///        SafeERC20 for all token transfers, checks-effects-interactions pattern,
///        custom errors for gas efficiency and clarity.
contract PacificaCarryVault is ERC4626, ReentrancyGuard, AccessControl {
    using SafeERC20 for IERC20;
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    // ── Roles ───────────────────────────────────────────────────────────

    /// @notice Role identifier for the bot operator (NAV signer).
    bytes32 public constant OPERATOR_ROLE = keccak256("OPERATOR_ROLE");

    /// @notice Role identifier for the guardian (pause authority + key rotation).
    bytes32 public constant GUARDIAN_ROLE = keccak256("GUARDIAN_ROLE");

    // ── Allocation ──────────────────────────────────────────────────────

    /// @notice Fraction of each deposit (in basis points) routed to the
    ///         treasury lending market for base yield. The remainder stays
    ///         as idle USDC for the off-chain perp executor.
    uint256 public constant TREASURY_RATIO_BPS = 3000; // 30%

    /// @notice Basis points denominator (10000 = 100%).
    uint256 public constant BPS_DENOMINATOR = 10_000;

    /// @notice Fee charged on the instant redeem path, in basis points.
    /// @dev Fixed at 5 bps (0.05%) by design. Not upgradeable via a setter —
    ///      a fee change requires a vault redeploy.
    uint16 public constant INSTANT_WITHDRAW_FEE_BPS = 5;

    // ── Errors ──────────────────────────────────────────────────────────
    error VaultPaused();
    error VaultNotPaused();
    error ZeroAssets();
    error ZeroShares();
    error NotRequestOwner();
    error CooldownNotElapsed();
    error AlreadyClaimed();
    error WithdrawDisabled();
    error InvalidNavSignature();
    error StaleTimestamp();
    error NavDeltaTooLarge();
    error TreasuryMintFailed(uint256 errorCode);
    error TreasuryRedeemFailed(uint256 errorCode);
    error InsufficientLiquidity();
    error ZeroAddress();

    // ── Events ──────────────────────────────────────────────────────────

    /// @notice Emitted when a user requests a withdrawal.
    /// @param id Monotonically increasing request ID
    /// @param user Address of the requester
    /// @param shares Number of shares burned
    /// @param assets USDC amount locked for the claim
    event WithdrawRequested(uint256 indexed id, address user, uint256 shares, uint256 assets);

    /// @notice Emitted when a user claims a completed withdrawal.
    /// @param id The request ID being claimed
    /// @param user Address of the claimant
    /// @param assets USDC amount transferred out
    event WithdrawClaimed(uint256 indexed id, address user, uint256 assets);

    /// @notice Emitted when the operator submits a new NAV report.
    /// @param newNav Updated total assets value (USDC, 6 decimals)
    /// @param timestamp Unix timestamp of the report
    event NavReported(uint256 newNav, uint256 timestamp);

    /// @notice Emitted when the guardian pauses the vault.
    /// @param guardian Address of the guardian who paused
    event Paused(address indexed guardian);

    /// @notice Emitted when the guardian unpauses the vault.
    /// @param guardian Address of the guardian who unpaused
    event Unpaused(address indexed guardian);

    /// @notice Emitted when the operator key is rotated.
    /// @param oldOperator Address of the outgoing operator
    /// @param newOperator Address of the incoming operator
    event OperatorChanged(address indexed oldOperator, address indexed newOperator);

    /// @notice Emitted when the guardian key is rotated.
    /// @param oldGuardian Address of the outgoing guardian
    /// @param newGuardian Address of the incoming guardian
    event GuardianChanged(address indexed oldGuardian, address indexed newGuardian);

    /// @notice Emitted when USDC is supplied to the treasury lending market.
    /// @param amount USDC amount sent to the treasury
    event TreasuryDeposited(uint256 amount);

    /// @notice Emitted when USDC is redeemed from the treasury lending market.
    /// @param amount USDC amount pulled out of the treasury
    event TreasuryRedeemed(uint256 amount);

    /// @notice Emitted when a user (or wrapper) executes an instant redemption.
    /// @param caller Address that called instantRedeem
    /// @param shares Shares burned
    /// @param grossAssets Gross assets value of the burned shares (pre-fee)
    /// @param fee Fee transferred to feeRecipient
    event InstantRedeemed(address indexed caller, uint256 shares, uint256 grossAssets, uint256 fee);

    /// @notice Emitted when the guardian rotates the instant-redeem fee recipient.
    /// @param oldRecipient Previous fee recipient
    /// @param newRecipient New fee recipient
    event FeeRecipientChanged(address indexed oldRecipient, address indexed newRecipient);

    // ── Types ───────────────────────────────────────────────────────────

    /// @notice Represents a pending withdrawal in the queue.
    /// @param user Address that requested the withdrawal (and can claim it)
    /// @param assets USDC amount to pay out on claim
    /// @param unlockTimestamp Earliest block.timestamp at which claim is allowed
    /// @param claimed Whether this request has already been claimed
    struct WithdrawRequest {
        address user;
        uint256 assets;
        uint256 unlockTimestamp;
        bool claimed;
    }

    // ── State ───────────────────────────────────────────────────────────

    /// @notice Current operator address (NAV signer). Rotatable by guardian.
    address public operator;

    /// @notice Current guardian address (pause + key rotation). Rotatable by guardian.
    address public guardian;

    /// @notice Cooldown period for the withdraw queue, set at deploy time.
    /// @dev Immutable after construction. Slither: correctly marked immutable.
    uint256 public immutable cooldownSeconds;

    /// @notice Treasury lending market that holds the 30% base-yield allocation.
    /// @dev Immutable after construction. Compatible with real Moonwell at V2.
    IMoonwellMarket public immutable treasuryVault;

    /// @notice Whether the vault is currently paused (blocks deposits + claims).
    bool public paused;

    /// @notice Oracle-reported total assets. Updated by reportNAV, incremented
    ///         by deposit, decremented by claimWithdraw.
    uint256 public totalAssetsStored;

    /// @notice Next withdraw request ID (monotonically increasing counter).
    uint256 public nextRequestId;

    /// @notice Mapping from request ID to withdraw request details.
    mapping(uint256 => WithdrawRequest) public withdrawRequests;

    /// @notice Timestamp of the last accepted NAV report (monotonically increasing).
    uint256 public lastTimestamp;

    /// @notice Whether at least one NAV report has been submitted.
    ///         The first report skips the sanity guard delta check.
    bool public navInitialized;

    /// @notice Recipient of the 5 bps instant-redeem fee. Rotatable by guardian.
    address public feeRecipient;

    // ── Constructor ─────────────────────────────────────────────────────

    /// @notice Deploy the vault with the given configuration.
    /// @dev Grants OPERATOR_ROLE and GUARDIAN_ROLE but does NOT grant
    ///      DEFAULT_ADMIN_ROLE to anyone, preventing role escalation.
    /// @param _usdc Address of the USDC token (the vault's underlying asset)
    /// @param _treasuryVault Address of the Moonwell-style lending market
    ///        for the 30% base-yield allocation
    /// @param _operator Address granted OPERATOR_ROLE (bot key for NAV signing)
    /// @param _guardian Address granted GUARDIAN_ROLE (governance key for pause + rotation)
    /// @param _cooldownSeconds Withdraw queue cooldown in seconds (e.g. 86400 = 24h)
    /// @param _feeRecipient Initial recipient of the 5 bps instant-redeem fee
    constructor(
        IERC20 _usdc,
        IMoonwellMarket _treasuryVault,
        address _operator,
        address _guardian,
        uint256 _cooldownSeconds,
        address _feeRecipient
    )
        ERC20("Pacifica Carry Vault", "pcvUSDC")
        ERC4626(_usdc)
    {
        if (_feeRecipient == address(0)) revert ZeroAddress();
        treasuryVault = _treasuryVault;
        operator = _operator;
        guardian = _guardian;
        cooldownSeconds = _cooldownSeconds;
        feeRecipient = _feeRecipient;

        _grantRole(OPERATOR_ROLE, _operator);
        _grantRole(GUARDIAN_ROLE, _guardian);
    }

    // ── ERC-4626 overrides ──────────────────────────────────────────────

    /// @notice Returns the total assets managed by the vault.
    /// @dev Sums three buckets:
    ///      1. Idle USDC sitting in the vault (perp margin staging area)
    ///      2. Underlying USDC held in the treasury lending market (with accrued interest)
    ///      3. The off-chain perp margin slot (`totalAssetsStored`) reported by the operator
    ///      Together these capture the full NAV of the strategy at any point in time.
    /// @return Total assets in USDC (6 decimals)
    function totalAssets() public view override returns (uint256) {
        uint256 idle = IERC20(asset()).balanceOf(address(this));
        uint256 inTreasury = treasuryVault.balanceOfUnderlying(address(this));
        return idle + inTreasury + totalAssetsStored;
    }

    /// @notice Deposit USDC into the vault in exchange for shares.
    /// @dev Follows checks-effects-interactions: pause check, ERC4626 deposit
    ///      (which transfers USDC in and mints shares at the current price),
    ///      then split the new USDC 70/30 by sending 30% to the treasury
    ///      lending market. The remaining 70% stays as idle USDC for the
    ///      off-chain perp executor. Protected by ReentrancyGuard.
    ///      Note: `totalAssetsStored` is NOT modified here — it tracks only
    ///      the off-chain margin component, updated exclusively by reportNAV.
    ///      The deposited USDC is reflected in `totalAssets()` via the idle
    ///      balance and treasury balance.
    /// @param assets Amount of USDC to deposit (6 decimals)
    /// @param receiver Address that receives the minted shares
    /// @return shares Amount of vault shares minted to the receiver
    function deposit(uint256 assets, address receiver)
        public
        override
        nonReentrant
        returns (uint256 shares)
    {
        if (paused) revert VaultPaused();
        if (assets == 0) revert ZeroAssets();

        shares = super.deposit(assets, receiver);

        // Split: 30% to treasury, 70% remains idle for perp margin staging.
        uint256 toTreasury = (assets * TREASURY_RATIO_BPS) / BPS_DENOMINATOR;
        if (toTreasury > 0) {
            IERC20(asset()).forceApprove(address(treasuryVault), toTreasury);
            uint256 err = treasuryVault.mint(toTreasury);
            if (err != 0) revert TreasuryMintFailed(err);
            emit TreasuryDeposited(toTreasury);
        }
    }

    /// @notice Disabled — use requestWithdraw + claimWithdraw instead.
    /// @dev Always reverts. Standard ERC-4626 withdraw is replaced by the
    ///      two-step queue to enforce a cooldown period.
    function withdraw(uint256, address, address) public pure override returns (uint256) {
        revert WithdrawDisabled();
    }

    /// @notice Disabled — use requestWithdraw + claimWithdraw instead.
    /// @dev Always reverts. Standard ERC-4626 redeem is replaced by the
    ///      two-step queue to enforce a cooldown period.
    function redeem(uint256, address, address) public pure override returns (uint256) {
        revert WithdrawDisabled();
    }

    /// @notice Disabled — use deposit(assets, receiver) instead.
    /// @dev Always reverts. Only deposit() is the supported entry point.
    function mint(uint256, address) public pure override returns (uint256) {
        revert WithdrawDisabled();
    }

    // ── Instant redeem (liquid buffer, 5 bps fee) ───────────────────────

    /// @notice Instant redemption from the liquid USDC buffer.
    /// @dev Burns shares, charges a 5 bps fee, and transfers the net USDC
    ///      to the caller in a single tx. Served exclusively from idle USDC
    ///      — treasury redemption is intentionally NOT triggered on this
    ///      path (Instant is the fast lane; slow unwinds go through the
    ///      requestWithdraw/claimWithdraw queue). If idle USDC is insufficient
    ///      to cover the gross assets, reverts with InsufficientLiquidity so
    ///      the frontend can route the user to the Scheduled path.
    ///      Allowed even when `navInitialized == false` because `totalAssets()`
    ///      reads idle + treasury live.
    ///      Checks-effects-interactions: read idle, check, burn shares, pay fee,
    ///      pay user. Protected by ReentrancyGuard.
    /// @param shares Amount of vault shares to burn (caller must own them)
    /// @return assetsOut Net USDC transferred to the caller (gross minus fee)
    function instantRedeem(uint256 shares) external nonReentrant returns (uint256 assetsOut) {
        if (paused) revert VaultPaused();
        if (shares == 0) revert ZeroShares();

        uint256 gross = convertToAssets(shares);
        uint256 fee = (gross * INSTANT_WITHDRAW_FEE_BPS) / BPS_DENOMINATOR;
        assetsOut = gross - fee;

        // Hard fail-fast: instant path is served from idle USDC only.
        uint256 idle = IERC20(asset()).balanceOf(address(this));
        if (idle < gross) revert InsufficientLiquidity();

        // Effects
        _burn(msg.sender, shares);

        // Interactions
        if (fee > 0) {
            IERC20(asset()).safeTransfer(feeRecipient, fee);
        }
        IERC20(asset()).safeTransfer(msg.sender, assetsOut);

        emit InstantRedeemed(msg.sender, shares, gross, fee);
    }

    // ── Withdraw queue ──────────────────────────────────────────────────

    /// @notice Request a withdrawal. Burns shares immediately and creates
    ///         a claim ticket with a cooldown.
    /// @dev Shares are burned at the current share price. The resulting asset
    ///      amount is locked in the queue. The caller can claim after
    ///      cooldownSeconds have elapsed. Allowed even when paused so users
    ///      can always signal intent to exit.
    /// @param shares Number of shares to burn (must be > 0, caller must own them)
    /// @return requestId Monotonically increasing request ID
    function requestWithdraw(uint256 shares) external nonReentrant returns (uint256 requestId) {
        if (shares == 0) revert ZeroShares();

        // Checks: caller must own enough shares (ERC20._burn reverts otherwise)
        // Effects: compute assets at current share price, burn shares, record request
        uint256 assets = convertToAssets(shares);
        _burn(msg.sender, shares);

        requestId = nextRequestId++;
        withdrawRequests[requestId] = WithdrawRequest({
            user: msg.sender,
            assets: assets,
            unlockTimestamp: block.timestamp + cooldownSeconds,
            claimed: false
        });

        emit WithdrawRequested(requestId, msg.sender, shares, assets);
    }

    /// @notice Claim a previously requested withdrawal after cooldown.
    /// @dev Follows checks-effects-interactions: verifies ownership, cooldown,
    ///      and single-use, then marks claimed, redeems from the treasury
    ///      lending market if idle USDC is insufficient, then transfers USDC
    ///      out. Protected by ReentrancyGuard.
    ///      `totalAssetsStored` is NOT decremented here — it represents the
    ///      off-chain margin component which is unaffected by claims pulled
    ///      from idle/treasury. The total NAV decreases naturally because
    ///      `totalAssets()` reads idle and treasury balances live.
    /// @param requestId The ID returned by requestWithdraw
    /// @return assets Amount of USDC transferred to the caller
    // slither-disable-next-line reentrancy-no-eth,reentrancy-benign,reentrancy-balance
    function claimWithdraw(uint256 requestId) external nonReentrant returns (uint256 assets) {
        if (paused) revert VaultPaused();

        WithdrawRequest storage req = withdrawRequests[requestId];

        // Checks
        if (req.user != msg.sender) revert NotRequestOwner();
        if (block.timestamp < req.unlockTimestamp) revert CooldownNotElapsed();
        if (req.claimed) revert AlreadyClaimed();

        // Effects
        assets = req.assets;
        req.claimed = true;

        // Interactions: ensure enough idle USDC, redeeming from treasury if needed.
        // The reentrancy-balance detector flags `idle` as a balance read before
        // an external call. This is safe because (a) `nonReentrant` guards the
        // entire function, (b) `treasuryVault` is immutable and trusted, and
        // (c) the post-call `err != 0` check uses the return value of the call,
        // not stale state.
        uint256 idle = IERC20(asset()).balanceOf(address(this));
        if (idle < assets) {
            uint256 needed = assets - idle;
            uint256 treasuryBal = treasuryVault.balanceOfUnderlying(address(this));
            if (treasuryBal < needed) revert InsufficientLiquidity();
            uint256 err = treasuryVault.redeem(needed);
            if (err != 0) revert TreasuryRedeemFailed(err);
            emit TreasuryRedeemed(needed);
        }

        IERC20(asset()).safeTransfer(msg.sender, assets);

        emit WithdrawClaimed(requestId, msg.sender, assets);
    }

    // ── NAV Reporter ────────────────────────────────────────────────────

    /// @notice Submit a signed NAV report to update the off-chain perp margin slot.
    /// @dev Updates `totalAssetsStored`, which represents the off-chain perp
    ///      margin component of the vault's total NAV. The treasury and idle
    ///      buckets are tracked separately and live (no oracle needed).
    ///      The signing payload must be exactly:
    ///      keccak256(abi.encodePacked(
    ///        "PACIFICA_CARRY_VAULT_NAV",
    ///        address(vault),
    ///        uint256(newNav),
    ///        uint256(timestamp)
    ///      ))
    ///      signed via EIP-191 personal_sign by the current operator.
    ///      The sanity guard rejects any single report that moves NAV by >= 10%.
    ///      The first report after deployment skips the delta check.
    ///      Allowed even when paused so the oracle stays current for accurate
    ///      pricing when the vault resumes.
    /// @custom:security Only the operator key can sign valid NAV reports. The
    ///      10% sanity guard limits damage from a compromised key to at most
    ///      10% per report. Timestamp monotonicity prevents replay attacks.
    /// @param newNav New total assets value in USDC (6 decimals)
    /// @param timestamp Unix seconds, must be strictly greater than lastTimestamp
    /// @param signature EIP-191 personal_sign signature from the operator
    function reportNAV(
        uint256 newNav,
        uint256 timestamp,
        bytes calldata signature
    ) external nonReentrant {
        // 1. Recover signer from the exact payload hash specified in INTERFACES.md §3
        bytes32 payloadHash = keccak256(
            abi.encodePacked(
                "PACIFICA_CARRY_VAULT_NAV",
                address(this),
                newNav,
                timestamp
            )
        );
        bytes32 ethSignedHash = payloadHash.toEthSignedMessageHash();
        address signer = ethSignedHash.recover(signature);

        // 2. Signer must be the current operator
        if (signer != operator) revert InvalidNavSignature();

        // 3. Timestamp must be strictly monotonic
        if (timestamp <= lastTimestamp) revert StaleTimestamp();

        // 4. Sanity guard: |newNav - lastNav| * 10 < lastNav (strict 10%)
        //    First report skips the delta check.
        if (navInitialized) {
            uint256 lastNav = totalAssetsStored;
            uint256 delta = newNav > lastNav ? newNav - lastNav : lastNav - newNav;
            if (delta * 10 >= lastNav) revert NavDeltaTooLarge();
        }

        // Effects: update oracle slot and timestamp
        totalAssetsStored = newNav;
        lastTimestamp = timestamp;
        navInitialized = true;

        emit NavReported(newNav, timestamp);
    }

    /// @notice Returns the share price scaled to 1e18.
    /// @dev If totalSupply is 0 (no shares outstanding), returns 1e18
    ///      (1:1 initial price) to prevent division by zero and to set the
    ///      first depositor's entry price. Otherwise derives from `totalAssets()`
    ///      which sums idle USDC + treasury balance + reported margin.
    /// @return The share price: totalAssets() * 1e18 / totalSupply
    function sharePrice() external view returns (uint256) {
        // Strict equality with 0 is the canonical divide-by-zero guard for
        // an empty vault. There is no off-by-one risk because totalSupply
        // is an integer.
        uint256 supply = totalSupply();
        // slither-disable-next-line incorrect-equality
        if (supply == 0) return 1e18;
        return (totalAssets() * 1e18) / supply;
    }

    // ── Access Control ─────────────────────────────────────────────────

    /// @notice Pause the vault. Blocks deposits and claim withdrawals.
    /// @dev requestWithdraw and reportNAV remain available during pause.
    ///      This allows users to queue exits and the oracle to stay current.
    /// @custom:security Only GUARDIAN_ROLE. Reverts if already paused to
    ///      prevent misleading duplicate Paused events.
    function pause() external onlyRole(GUARDIAN_ROLE) {
        if (paused) revert VaultPaused();
        paused = true;
        emit Paused(msg.sender);
    }

    /// @notice Unpause the vault. Re-enables deposits and claim withdrawals.
    /// @custom:security Only GUARDIAN_ROLE. Reverts if not paused.
    function unpause() external onlyRole(GUARDIAN_ROLE) {
        if (!paused) revert VaultNotPaused();
        paused = false;
        emit Unpaused(msg.sender);
    }

    /// @notice Rotate the operator key. The new operator becomes the NAV signer.
    /// @dev Revokes OPERATOR_ROLE from the old operator and grants it to the new one.
    ///      Old operator's pending NAV signatures become invalid immediately.
    /// @custom:security Only GUARDIAN_ROLE. No zero-address check — setting operator
    ///      to address(0) effectively disables NAV reporting (acceptable emergency action).
    /// @param newOperator Address of the new operator
    function setOperator(address newOperator) external onlyRole(GUARDIAN_ROLE) {
        address oldOperator = operator;
        _revokeRole(OPERATOR_ROLE, oldOperator);
        operator = newOperator;
        _grantRole(OPERATOR_ROLE, newOperator);
        emit OperatorChanged(oldOperator, newOperator);
    }

    /// @notice Rotate the guardian key. Transfers pause authority to the new guardian.
    /// @dev The old guardian loses GUARDIAN_ROLE and can no longer pause/unpause
    ///      or rotate keys. This is a single-key model; multisig is future work.
    /// @custom:security Only GUARDIAN_ROLE. No zero-address check — setting guardian
    ///      to address(0) would permanently lock the guardian role (irreversible).
    ///      This is documented as a known risk in SECURITY.md.
    /// @notice Rotate the instant-redeem fee recipient.
    /// @dev Only GUARDIAN_ROLE. Rejects zero-address to prevent accidentally
    ///      burning fee revenue.
    /// @param newRecipient Address of the new fee recipient
    function setFeeRecipient(address newRecipient) external onlyRole(GUARDIAN_ROLE) {
        if (newRecipient == address(0)) revert ZeroAddress();
        address oldRecipient = feeRecipient;
        feeRecipient = newRecipient;
        emit FeeRecipientChanged(oldRecipient, newRecipient);
    }

    /// @param newGuardian Address of the new guardian
    function setGuardian(address newGuardian) external onlyRole(GUARDIAN_ROLE) {
        address oldGuardian = guardian;
        _revokeRole(GUARDIAN_ROLE, oldGuardian);
        guardian = newGuardian;
        _grantRole(GUARDIAN_ROLE, newGuardian);
        emit GuardianChanged(oldGuardian, newGuardian);
    }

    /// @notice ERC-165 interface support for AccessControl.
    /// @param interfaceId The 4-byte interface identifier (ERC-165)
    /// @return True if the contract implements the requested interface
    function supportsInterface(bytes4 interfaceId)
        public
        view
        override(AccessControl)
        returns (bool)
    {
        return super.supportsInterface(interfaceId);
    }
}
