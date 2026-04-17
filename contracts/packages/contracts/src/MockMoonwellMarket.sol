// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IMoonwellMarket} from "./IMoonwellMarket.sol";

/// @title MockMoonwellMarket
/// @notice A mock Compound v2 / Moonwell-style lending market that pays a flat
///         5% APY (simple interest) on supplied USDC. Used for V1.5 treasury
///         layer testing on testnet. The interface matches the real Moonwell
///         contract so the vault can swap to the real market at V2 with zero
///         changes.
/// @dev Each account tracks a `principal` (underlying balance) and a
///      `lastUpdate` timestamp. Interest accrues linearly from `lastUpdate`
///      until the next interaction (mint, redeem, or balance query).
///      Interest is realized into principal on every state-changing call.
contract MockMoonwellMarket is IMoonwellMarket {
    using SafeERC20 for IERC20;

    /// @notice The underlying token (USDC, 6 decimals).
    IERC20 public immutable underlying;

    /// @notice 5% APY expressed as basis points per year.
    uint256 public constant APY_BPS = 500;

    /// @notice Basis points denominator.
    uint256 public constant BPS_DENOMINATOR = 10_000;

    /// @notice Number of seconds in a year (365 days).
    uint256 public constant SECONDS_PER_YEAR = 365 days;

    error ZeroAmount();
    error InsufficientBalance();

    /// @notice Per-account principal and last accrual timestamp.
    struct Account {
        uint256 principal;
        uint256 lastUpdate;
    }

    mapping(address => Account) internal accounts;

    /// @param _underlying Address of the underlying ERC20 (USDC)
    constructor(IERC20 _underlying) {
        underlying = _underlying;
    }

    /// @notice Supply USDC to the market and accrue 5% APY going forward.
    /// @dev Realizes any pending interest into the caller's principal first,
    ///      then adds the new amount and pulls USDC via transferFrom.
    /// @param amount Amount of USDC to supply (must be > 0)
    /// @return 0 on success (Compound convention)
    function mint(uint256 amount) external returns (uint256) {
        if (amount == 0) revert ZeroAmount();

        Account storage acc = accounts[msg.sender];
        _accrue(acc);
        acc.principal += amount;

        underlying.safeTransferFrom(msg.sender, address(this), amount);
        return 0;
    }

    /// @notice Redeem USDC from the market.
    /// @dev Realizes pending interest into principal, then deducts the
    ///      redeem amount and transfers USDC out.
    /// @param amount Amount of USDC to redeem
    /// @return 0 on success (Compound convention)
    function redeem(uint256 amount) external returns (uint256) {
        if (amount == 0) revert ZeroAmount();

        Account storage acc = accounts[msg.sender];
        _accrue(acc);

        if (acc.principal < amount) revert InsufficientBalance();
        acc.principal -= amount;

        underlying.safeTransfer(msg.sender, amount);
        return 0;
    }

    /// @notice Returns the underlying balance of `account` with accrued interest.
    /// @dev View function — computes interest without modifying state.
    /// @param account Address to query
    /// @return Principal plus accrued interest since last update
    function balanceOfUnderlying(address account) external view returns (uint256) {
        Account memory acc = accounts[account];
        if (acc.principal == 0) return 0;
        uint256 elapsed = block.timestamp - acc.lastUpdate;
        uint256 interest = (acc.principal * APY_BPS * elapsed) / (BPS_DENOMINATOR * SECONDS_PER_YEAR);
        return acc.principal + interest;
    }

    /// @notice Returns the cached exchange rate (constant 1e18 in this mock).
    /// @return Always 1e18 (we don't simulate exchange rate complexity)
    function exchangeRateStored() external pure returns (uint256) {
        return 1e18;
    }

    /// @notice Returns the raw principal (without accrued interest) for `account`.
    /// @dev Useful for tests and observers; not part of the IMoonwellMarket interface.
    function principalOf(address account) external view returns (uint256) {
        return accounts[account].principal;
    }

    /// @dev Realize accrued interest into principal and update lastUpdate.
    function _accrue(Account storage acc) internal {
        if (acc.principal > 0) {
            uint256 elapsed = block.timestamp - acc.lastUpdate;
            uint256 interest = (acc.principal * APY_BPS * elapsed) / (BPS_DENOMINATOR * SECONDS_PER_YEAR);
            acc.principal += interest;
        }
        acc.lastUpdate = block.timestamp;
    }
}
