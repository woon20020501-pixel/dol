// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IMoonwellMarket
/// @notice Minimal interface for a Compound v2 / Moonwell-style lending market.
///         Compatible with the real Moonwell market on Base mainnet, allowing
///         the vault to swap the mock for the real contract at V2 with zero
///         interface changes.
/// @dev `mint` and `redeem` follow the Compound convention of returning 0 on
///      success and a non-zero error code on failure.
interface IMoonwellMarket {
    /// @notice Supply `amount` of underlying tokens and receive mTokens.
    /// @param amount Amount of underlying (USDC) to supply
    /// @return error 0 on success, non-zero error code otherwise
    function mint(uint256 amount) external returns (uint256 error);

    /// @notice Redeem `amount` of underlying tokens by burning the equivalent mTokens.
    /// @param amount Amount of underlying (USDC) to redeem
    /// @return error 0 on success, non-zero error code otherwise
    function redeem(uint256 amount) external returns (uint256 error);

    /// @notice The current underlying balance of `account`, including accrued interest.
    /// @param account Address to query
    /// @return The underlying token balance with accrued interest
    function balanceOfUnderlying(address account) external view returns (uint256);

    /// @notice The cached exchange rate from mTokens to underlying, scaled by 1e18.
    /// @return The exchange rate (mock returns a constant 1e18)
    function exchangeRateStored() external view returns (uint256);
}
