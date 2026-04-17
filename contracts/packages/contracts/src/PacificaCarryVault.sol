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
///      - Access: OPERATOR_ROLE (bot) can sign NAV reports. GUARDIAN_ROLE (PM)
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
    /// @dev Fixed at 5 bps (0.05%) per policy. Not upgradeable via
    ///      setter — a fee change requires a vault redeploy.
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
    /// @notice Thrown when a NAV report arrives before `minReportInterval`
    ///         seconds have elapsed since the previous report.
    /// @dev Added 2026-04-17 (C4 hardening). Prevents a compromised operator
    ///      from chaining many small reports to compound drift beyond the
    ///      per-report 10% guard. Benchmarked against MakerDAO OSM (1h
    ///      `hop`) and Chainlink price-feed heartbeat floors (3600s on
    ///      BTC/USD, ETH/USD). Disabled when `minReportInterval == 0`
    ///      (backward-compatible testing mode).
    error NavReportTooFrequent();
    /// @notice Thrown when a NAV report would push the cumulative 24h
    ///         delta above `maxDailyDeltaBps`.
    /// @dev Added 2026-04-17 (C4 hardening). Modeled after Lido's
    ///      `OracleReportSanityChecker` `annualBalanceIncreaseBPLimit` and
    ///      `oneOffCLBalanceDecreaseBPLimit` pattern — a per-calendar-day
    ///      cumulative cap that bounds maximum drain rate under operator
    ///      compromise. Disabled when `maxDailyDeltaBps == 0`.
    error NavDailyDeltaExceeded();

    /// @notice Thrown on EIP-7540 claim/request when the caller is neither
    ///         the controller nor an approved operator.
    error NotControllerOrOperator();

    /// @notice Thrown when the caller attempts to claim a share amount that
    ///         does not match any single pending request of the controller.
    /// @dev This implementation restricts EIP-7540 `redeem(shares, ...)` to
    ///      per-request whole-claim semantics (no partial claims, no
    ///      aggregation across multiple requests). Rationale: simpler state
    ///      machine + lower gas; the Centrifuge reference impl. aggregates
    ///      across requests but at meaningful gas cost. Most users claim
    ///      single requests anyway.
    error AsyncClaimShareMismatch();

    /// @notice Thrown when an EIP-7540 async claim finds no matching ready
    ///         request for the controller.
    error AsyncClaimNoReadyRequest();

    // ── Events ──────────────────────────────────────────────────────────

    /// @notice Emitted when a user requests a withdrawal.
    /// @dev Both `id` and `user` are indexed for subgraph / indexer
    ///      efficiency — common queries are "state of request N" and
    ///      "all requests by user X". Adding `indexed` moves these
    ///      args from data into topics; does not change the event
    ///      signature hash (topic0).
    /// @param id Monotonically increasing request ID
    /// @param user Address of the requester
    /// @param shares Number of shares burned
    /// @param assets USDC amount locked for the claim
    event WithdrawRequested(uint256 indexed id, address indexed user, uint256 shares, uint256 assets);

    /// @notice Emitted when a user claims a completed withdrawal.
    /// @dev Both `id` and `user` indexed (same rationale as `WithdrawRequested`).
    /// @param id The request ID being claimed
    /// @param user Address of the claimant
    /// @param assets USDC amount transferred out
    event WithdrawClaimed(uint256 indexed id, address indexed user, uint256 assets);

    /// @notice Emitted when the operator submits a new NAV report.
    /// @dev `timestamp` is indexed so monitoring clients can query the
    ///      most recent N reports via topic filters without full log scan.
    /// @param newNav Updated total assets value (USDC, 6 decimals)
    /// @param timestamp Unix timestamp of the report
    event NavReported(uint256 newNav, uint256 indexed timestamp);

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

    // ── ERC-7540 events (EIP-7540 §Events) ─────────────────────────────

    /// @notice EIP-7540 `RedeemRequest` event (shares-denominated).
    /// @dev Emitted alongside legacy `WithdrawRequested` so that EIP-7540
    ///      compliant indexers (Centrifuge subgraph, Enzyme, etc.) can
    ///      track pending requests without relying on the legacy event.
    /// @param controller Address authorized to claim the request
    /// @param owner Address whose shares were burned to create the request
    /// @param requestId The request ID (monotonic in this implementation)
    /// @param sender The `msg.sender` of the `requestRedeem` call
    /// @param shares Amount of shares burned (EIP-7540 unit of account)
    event RedeemRequest(
        address indexed controller,
        address indexed owner,
        uint256 indexed requestId,
        address sender,
        uint256 shares
    );

    /// @notice EIP-7540 `OperatorSet` event.
    event OperatorSet(
        address indexed controller,
        address indexed operator,
        bool approved
    );

    // ── Types ───────────────────────────────────────────────────────────

    /// @notice Represents a pending withdrawal in the queue.
    /// @dev EIP-7540 compliance note: this implementation locks BOTH `assets`
    ///      (spec-optional, used by our cooldown-priced claim path) AND
    ///      `shares` (spec-mandatory, returned by `pendingRedeemRequest` /
    ///      `claimableRedeemRequest`). Spec says shares are the canonical
    ///      unit of account; `assets` here is an implementation-specific
    ///      locked-price field to avoid H1 bank-run asymmetry during the
    ///      cooldown window. See `docs/AUDIT_PREP.md §1.2` for the v2 plan
    ///      that will drop `assets` and price at claim time per EIP-7540
    ///      canonical semantics.
    /// @param user Address that requested the withdrawal (and can claim it).
    ///        Named `controller` per EIP-7540 terminology, but the field is
    ///        `user` for backward compatibility with the v1 event log.
    /// @param assets USDC amount to pay out on claim (cooldown-locked price)
    /// @param shares Shares that were burned to create the request (EIP-7540)
    /// @param unlockTimestamp Earliest block.timestamp at which claim is allowed
    /// @param claimed Whether this request has already been claimed
    struct WithdrawRequest {
        address user;
        uint256 assets;
        uint256 shares;
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

    // ── NAV rate-limit state (C4 hardening, 2026-04-17) ────────────────

    /// @notice Minimum seconds that must elapse between two accepted
    ///         `reportNAV` calls.
    /// @dev Set at construction. Zero disables the check (useful for
    ///      testing; production should set >= 1 hour per MakerDAO OSM /
    ///      Chainlink heartbeat convention).
    uint256 public immutable minReportInterval;

    /// @notice Maximum cumulative |delta| allowed in a single UTC day,
    ///         expressed in basis points of `totalAssetsStored` at the
    ///         start of each report.
    /// @dev Set at construction. Zero disables the check. Modeled after
    ///      Lido `OracleReportSanityChecker` daily caps. A value of 100
    ///      (1%) caps attacker drain to ~365%/year even with a compromised
    ///      operator, vs. unbounded compound drift in the unprotected
    ///      10%-per-report model.
    uint256 public immutable maxDailyDeltaBps;

    /// @notice Per-UTC-day cumulative |delta| consumed by accepted NAV
    ///         reports. Keyed by `block.timestamp / 1 days`.
    mapping(uint256 => uint256) private _dailyDeltaAccumulated;

    // ── ERC-7540 state (EIP-7540 §Operator methods) ────────────────────

    /// @notice `_operators[controller][operator] = approved`.
    ///         Per EIP-7540, an approved operator may perform request,
    ///         claim, and cancel actions on behalf of the controller.
    mapping(address => mapping(address => bool)) private _operators;

    /// @notice Append-only list of request IDs per controller, used by
    ///         the EIP-7540 `redeem(shares, receiver, controller)` claim
    ///         to find the matching ready request without requiring the
    ///         caller to pass the requestId explicitly.
    /// @dev Claimed requests are NOT removed (gas cost of shuffle > cost of
    ///      scan). The linear scan in `_findReadyRequestByShares` skips
    ///      claimed entries.
    mapping(address => uint256[]) private _controllerRequestIds;

    // ── Constructor ─────────────────────────────────────────────────────

    /// @notice Deploy the vault with the given configuration.
    /// @dev Grants OPERATOR_ROLE and GUARDIAN_ROLE but does NOT grant
    ///      DEFAULT_ADMIN_ROLE to anyone, preventing role escalation.
    /// @param _usdc Address of the USDC token (the vault's underlying asset)
    /// @param _treasuryVault Address of the Moonwell-style lending market
    ///        for the 30% base-yield allocation
    /// @param _operator Address granted OPERATOR_ROLE (bot key for NAV signing)
    /// @param _guardian Address granted GUARDIAN_ROLE (policy key for pause + rotation)
    /// @param _cooldownSeconds Withdraw queue cooldown in seconds (e.g. 86400 = 24h)
    /// @param _feeRecipient Initial recipient of the 5 bps instant-redeem fee
    /// @param _minReportInterval Minimum seconds between accepted NAV reports
    ///        (C4 hardening). Mainnet recommendation: 3600 (matches
    ///        MakerDAO OSM `hop` and Chainlink BTC/ETH heartbeat floors).
    ///        0 disables the check.
    /// @param _maxDailyDeltaBps Cumulative |delta| ceiling per UTC day, bps
    ///        of `totalAssetsStored`. Mainnet recommendation: 100 (1%)
    ///        following Lido `OracleReportSanityChecker` daily-cap pattern.
    ///        0 disables the check.
    constructor(
        IERC20 _usdc,
        IMoonwellMarket _treasuryVault,
        address _operator,
        address _guardian,
        uint256 _cooldownSeconds,
        address _feeRecipient,
        uint256 _minReportInterval,
        uint256 _maxDailyDeltaBps
    ) ERC20("Pacifica Carry Vault", "pcvUSDC") ERC4626(_usdc) {
        if (_feeRecipient == address(0)) revert ZeroAddress();
        treasuryVault = _treasuryVault;
        operator = _operator;
        guardian = _guardian;
        cooldownSeconds = _cooldownSeconds;
        feeRecipient = _feeRecipient;
        minReportInterval = _minReportInterval;
        maxDailyDeltaBps = _maxDailyDeltaBps;

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
    function deposit(uint256 assets, address receiver) public override nonReentrant returns (uint256 shares) {
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

    /// @notice ERC-4626 + EIP-7540 `withdraw(assets, receiver, controller)`.
    /// @dev Post-B3 hardening: now implemented as an async-claim overload
    ///      that matches a single ready request of `controller` whose
    ///      locked `assets` value equals the `assets` argument. Caller
    ///      must be controller or approved operator. See `redeem(...)` for
    ///      the shares-keyed variant; both consume the same request queue
    ///      and both settle via `_processClaimTransfer`. This gives the
    ///      vault full EIP-7540 async-redeem compliance on both entrypoints.
    /// @param assets The exact locked-asset amount of a single ready request
    /// @param receiver USDC recipient
    /// @param controller Address whose claimable request is being consumed
    /// @return shares Shares that were burned when the request was created
    function withdraw(uint256 assets, address receiver, address controller)
        public
        override
        nonReentrant
        returns (uint256 shares)
    {
        if (paused) revert VaultPaused();
        if (assets == 0) revert ZeroAssets();
        if (msg.sender != controller && !_operators[controller][msg.sender]) {
            revert NotControllerOrOperator();
        }

        uint256 id = _findReadyRequestByAssets(controller, assets);
        WithdrawRequest storage req = withdrawRequests[id];
        req.claimed = true;
        shares = req.shares;

        _processClaimTransfer(assets, receiver);
        emit Withdraw(msg.sender, receiver, controller, assets, shares);
    }

    /// @dev Assets-keyed mirror of `_findReadyRequestByShares`.
    /// @dev Caller contract is responsible for ensuring `targetAssets > 0`.
    function _findReadyRequestByAssets(address controller, uint256 targetAssets)
        internal
        view
        returns (uint256 id)
    {
        uint256[] storage list = _controllerRequestIds[controller];
        uint256 n = list.length;
        for (uint256 i = 0; i < n; i++) {
            uint256 candidate = list[i];
            WithdrawRequest storage req = withdrawRequests[candidate];
            if (req.claimed) continue;
            if (req.assets != targetAssets) continue;
            if (block.timestamp < req.unlockTimestamp) continue;
            return candidate;
        }
        for (uint256 j = 0; j < n; j++) {
            WithdrawRequest storage r = withdrawRequests[list[j]];
            if (!r.claimed && block.timestamp >= r.unlockTimestamp) {
                revert AsyncClaimShareMismatch();
            }
        }
        revert AsyncClaimNoReadyRequest();
    }

    /// @notice EIP-7540 async-claim redeem. Consumes one matching ready
    ///         request owned by `controller` and pays out its locked
    ///         `assets` to `receiver`.
    /// @dev Matches the EIP-7540 `redeem(shares, receiver, controller)`
    ///      signature. Per the `AsyncClaimShareMismatch` error NatSpec,
    ///      this implementation requires `shares` to match exactly one
    ///      of the controller's ready requests. Partial claims and
    ///      cross-request aggregation are out of scope for v1.
    /// @param shares The exact `sharesRequested` of a single ready request
    /// @param receiver USDC recipient
    /// @param controller Address whose claimable request is being consumed
    /// @return assets USDC paid to receiver
    function redeem(uint256 shares, address receiver, address controller)
        public
        override
        nonReentrant
        returns (uint256 assets)
    {
        if (paused) revert VaultPaused();
        if (shares == 0) revert ZeroShares();
        if (msg.sender != controller && !_operators[controller][msg.sender]) {
            revert NotControllerOrOperator();
        }

        // Find the first ready request owned by controller with matching shares
        uint256 id = _findReadyRequestByShares(controller, shares);

        WithdrawRequest storage req = withdrawRequests[id];
        req.claimed = true;
        assets = req.assets;

        _processClaimTransfer(assets, receiver);

        // Emit standard ERC-4626 `Withdraw` event so sync-side integrators
        // observe the settlement. The EIP-7540 claim does not have its own
        // distinct event; reusing Withdraw is the spec-recommended path.
        emit Withdraw(msg.sender, receiver, controller, assets, shares);
    }

    /// @dev Internal helper: linear scan of `_controllerRequestIds[controller]`
    ///      for the first ready, unclaimed request whose `sharesRequested`
    ///      exactly matches `targetShares`. Reverts if none found.
    ///      Cost: O(n) where n is the controller's lifetime request count
    ///      (claimed requests are not pruned, but the scan skips them).
    ///      Mitigated by users having few concurrent outstanding requests.
    /// @dev Caller contract is responsible for ensuring `targetShares > 0`.
    function _findReadyRequestByShares(address controller, uint256 targetShares)
        internal
        view
        returns (uint256 id)
    {
        uint256[] storage list = _controllerRequestIds[controller];
        uint256 n = list.length;
        for (uint256 i = 0; i < n; i++) {
            uint256 candidate = list[i];
            WithdrawRequest storage req = withdrawRequests[candidate];
            if (req.claimed) continue;
            if (req.shares != targetShares) continue;
            if (block.timestamp < req.unlockTimestamp) continue;
            return candidate;
        }
        // No exact match. Distinguish "ready but wrong shares" (mismatch)
        // from "nothing ready at all" (no-ready) so callers can surface a
        // precise error.
        for (uint256 j = 0; j < n; j++) {
            WithdrawRequest storage r = withdrawRequests[list[j]];
            if (!r.claimed && block.timestamp >= r.unlockTimestamp) {
                revert AsyncClaimShareMismatch();
            }
        }
        revert AsyncClaimNoReadyRequest();
    }

    /// @dev Internal: processes the payout side of a claim, pulling from
    ///      idle USDC first and falling back to the treasury vault.
    ///      Shared by `claimWithdraw`, `redeem`, and `withdraw` 3-arg
    ///      overloads. Callers are responsible for the pause check; this
    ///      function does not duplicate it.
    /// @dev Rejects zero-address receiver defensively — USDC itself
    ///      reverts on `transfer(address(0), ...)`, but surfacing the
    ///      error from the vault gives a clearer selector for callers.
    function _processClaimTransfer(uint256 amount, address receiver) internal {
        if (receiver == address(0)) revert ZeroAddress();
        uint256 idle = IERC20(asset()).balanceOf(address(this));
        if (idle < amount) {
            uint256 needed = amount - idle;
            uint256 treasuryBal = treasuryVault.balanceOfUnderlying(address(this));
            if (treasuryBal < needed) revert InsufficientLiquidity();
            uint256 err = treasuryVault.redeem(needed);
            if (err != 0) revert TreasuryRedeemFailed(err);
            emit TreasuryRedeemed(needed);
        }
        IERC20(asset()).safeTransfer(receiver, amount);
    }

    /// @notice Disabled — use deposit(assets, receiver) instead.
    /// @dev Always reverts. Only deposit() is the supported entry point.
    ///      EIP-7540 async-deposit is not supported (we are an async-redeem-
    ///      only vault per EIP-7540 §"Deposit vs Redeem asymmetry").
    function mint(uint256, address) public pure override returns (uint256) {
        revert WithdrawDisabled();
    }

    // ── EIP-7540 interface layer ────────────────────────────────────────

    /// @notice EIP-7540 `requestRedeem(shares, controller, owner)`.
    /// @dev `controller` and `owner` may differ if `owner` has granted
    ///      ERC-20 allowance to `msg.sender`. For this release we require
    ///      `owner == msg.sender || owner has approved msg.sender` and
    ///      any `controller` set by sender.
    /// @param shares Shares to burn from owner
    /// @param controller Address authorized to claim the resulting request
    /// @param owner Address from which shares are burned
    /// @return requestId EIP-7540 request ID
    function requestRedeem(uint256 shares, address controller, address owner)
        external
        nonReentrant
        returns (uint256 requestId)
    {
        // EIP-7540 §"Methods": the caller must be either the owner or an
        // approved operator of the owner. As a compatibility extension we
        // also honor ERC-20 share allowance (consumed on use) so that
        // existing ERC-20 relayers that pre-date EIP-7540 continue to work.
        if (owner != msg.sender && !_operators[owner][msg.sender]) {
            // Neither controller-of-owner nor approved operator → require
            // ERC-20 allowance (pre-7540 compatibility path, consumed).
            _spendAllowance(owner, msg.sender, shares);
        }
        return _requestRedeemInternal(shares, controller, owner, msg.sender);
    }

    /// @notice EIP-7540 `pendingRedeemRequest(requestId, controller)`.
    /// @dev Returns `shares` if the request is pending (before cooldown
    ///      elapses). Returns 0 if the request is claimable, already claimed,
    ///      or not owned by `controller`.
    function pendingRedeemRequest(uint256 requestId, address controller)
        external
        view
        returns (uint256 shares)
    {
        WithdrawRequest storage req = withdrawRequests[requestId];
        if (req.user != controller) return 0;
        if (req.claimed) return 0;
        if (block.timestamp >= req.unlockTimestamp) return 0;
        return req.shares;
    }

    /// @notice EIP-7540 `claimableRedeemRequest(requestId, controller)`.
    /// @dev Returns `shares` if the request is ready to claim. Returns 0
    ///      otherwise.
    function claimableRedeemRequest(uint256 requestId, address controller)
        external
        view
        returns (uint256 shares)
    {
        WithdrawRequest storage req = withdrawRequests[requestId];
        if (req.user != controller) return 0;
        if (req.claimed) return 0;
        if (block.timestamp < req.unlockTimestamp) return 0;
        return req.shares;
    }

    /// @notice EIP-7540 `setOperator(operator, approved)` —
    ///         msg.sender is the controller.
    function setOperator(address op, bool approved) external returns (bool) {
        _operators[msg.sender][op] = approved;
        emit OperatorSet(msg.sender, op, approved);
        return true;
    }

    /// @notice EIP-7540 `isOperator(controller, operator)`.
    function isOperator(address controller, address op) external view returns (bool) {
        return _operators[controller][op];
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
        return _requestRedeemInternal(shares, msg.sender, msg.sender, msg.sender);
    }

    /// @dev Shared request-creation path used by both the legacy
    ///      `requestWithdraw(shares)` entrypoint and the EIP-7540
    ///      `requestRedeem(shares, controller, owner)` entrypoint.
    ///      Emits BOTH the legacy `WithdrawRequested` event and the
    ///      EIP-7540 `RedeemRequest` event so indexers of either
    ///      schema can reconstruct state.
    /// @param shares Shares to burn from `owner`
    /// @param controller EIP-7540 controller — the address authorized to
    ///        claim. In the legacy path, controller == owner == sender.
    /// @param owner Address whose shares are burned
    /// @param sender The `msg.sender` of the outer call (for event fidelity)
    function _requestRedeemInternal(
        uint256 shares,
        address controller,
        address owner,
        address sender
    ) internal returns (uint256 requestId) {
        if (shares == 0) revert ZeroShares();

        // Checks: `owner` must own enough shares (ERC20._burn reverts otherwise)
        // Effects: compute assets at current share price, burn shares, record request
        uint256 assets = convertToAssets(shares);
        _burn(owner, shares);

        requestId = nextRequestId++;
        withdrawRequests[requestId] = WithdrawRequest({
            user: controller,
            assets: assets,
            shares: shares,
            unlockTimestamp: block.timestamp + cooldownSeconds,
            claimed: false
        });
        _controllerRequestIds[controller].push(requestId);

        emit WithdrawRequested(requestId, controller, shares, assets);
        emit RedeemRequest(controller, owner, requestId, sender, shares);
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

        // Interactions: shared with EIP-7540 `redeem`/`withdraw` async-claim
        // overloads via `_processClaimTransfer`. The helper pulls from idle
        // first then falls back to treasury redemption, then safe-transfers
        // to the receiver. Protected by the outer `nonReentrant` modifier.
        _processClaimTransfer(assets, msg.sender);

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
    function reportNAV(uint256 newNav, uint256 timestamp, bytes calldata signature) external nonReentrant {
        // 1. Recover signer from the exact payload hash specified in INTERFACES.md §3
        bytes32 payloadHash = keccak256(abi.encodePacked("PACIFICA_CARRY_VAULT_NAV", address(this), newNav, timestamp));
        bytes32 ethSignedHash = payloadHash.toEthSignedMessageHash();
        address signer = ethSignedHash.recover(signature);

        // 2. Signer must be the current operator
        if (signer != operator) revert InvalidNavSignature();

        // 3. Timestamp must be strictly monotonic
        if (timestamp <= lastTimestamp) revert StaleTimestamp();

        // 4. Minimum inter-report interval (C4 hardening). Disabled if 0.
        //    Skipped on the first report (lastTimestamp == 0) because
        //    the constraint is "no two reports closer than X seconds",
        //    which is only defined when a prior report exists.
        //    Empirical baseline: MakerDAO OSM `hop = 3600s`;
        //    Chainlink BTC/USD, ETH/USD heartbeat = 3600s.
        if (
            minReportInterval != 0 && navInitialized
                && timestamp < lastTimestamp + minReportInterval
        ) {
            revert NavReportTooFrequent();
        }

        // 5. Per-report sanity guard: |newNav - lastNav| * 10 < lastNav (strict 10%)
        //    First report skips the delta check.
        uint256 lastNav = totalAssetsStored;
        uint256 delta = 0;
        if (navInitialized) {
            delta = newNav > lastNav ? newNav - lastNav : lastNav - newNav;
            if (delta * 10 >= lastNav) revert NavDeltaTooLarge();
        }

        // 6. Daily cumulative cap (C4 hardening). Disabled if 0. Modeled on
        //    Lido `OracleReportSanityChecker.annualBalanceIncreaseBPLimit`.
        //    Uses UTC-day buckets; the bucket is keyed by the *report*
        //    timestamp, not `block.timestamp`, so operators cannot evade
        //    the cap by stalling the tx.
        if (maxDailyDeltaBps != 0 && navInitialized) {
            uint256 day = timestamp / 1 days;
            uint256 newDaySum = _dailyDeltaAccumulated[day] + delta;
            if (newDaySum * 10_000 > lastNav * maxDailyDeltaBps) {
                revert NavDailyDeltaExceeded();
            }
            _dailyDeltaAccumulated[day] = newDaySum;
        }

        // Effects: update oracle slot and timestamp
        totalAssetsStored = newNav;
        lastTimestamp = timestamp;
        navInitialized = true;

        emit NavReported(newNav, timestamp);
    }

    /// @notice View: cumulative |delta| already consumed on a given UTC day.
    /// @dev Useful for off-chain monitoring to pre-check whether the next
    ///      proposed NAV would exceed the daily cap.
    /// @param day UTC day index (i.e. `timestamp / 86400`)
    /// @return consumed Sum of |delta| accepted that day, in the same
    ///         units as `totalAssetsStored` (USDC 6 decimals)
    function dailyDeltaConsumed(uint256 day) external view returns (uint256 consumed) {
        return _dailyDeltaAccumulated[day];
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

    /// @notice ERC-165 interface support: AccessControl + EIP-7540 flags.
    /// @dev EIP-7540 interface IDs (empirically verified against Centrifuge
    ///      reference, EIP text, 2025-10):
    ///      - 0x620ee8e4 : IERC7540Redeem   (async redeem methods)
    ///      - 0xe3bc4e65 : IERC7540Operator (setOperator / isOperator)
    ///      - 0x2f0a18c5 : IERC7575         (multi-asset base; we only have one asset)
    ///      We return true for 7540-Redeem and 7540-Operator; 7540-Deposit
    ///      (async deposit) is deliberately unsupported — deposits in this
    ///      vault are synchronous.
    /// @param interfaceId The 4-byte interface identifier (ERC-165)
    /// @return True if the contract implements the requested interface
    function supportsInterface(bytes4 interfaceId) public view override(AccessControl) returns (bool) {
        // EIP-7540 Redeem (async redeem request + claim)
        if (interfaceId == 0x620ee8e4) return true;
        // EIP-7540 Operator (setOperator / isOperator)
        if (interfaceId == 0xe3bc4e65) return true;
        return super.supportsInterface(interfaceId);
    }
}
