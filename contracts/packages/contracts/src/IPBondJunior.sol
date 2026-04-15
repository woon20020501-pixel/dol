// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IPBondJunior
/// @notice Minimal interface for Senior -> Junior cross-calls during yield
///         distribution and loss absorption. Keeps the dependency one-way
///         (Senior imports this interface, Junior implements it).
interface IPBondJunior {
    /// @notice Transfer vault shares from Junior to Senior to cover a loss.
    ///         Called by Senior's distributeYield when Senior value < principal.
    /// @param vaultShares Number of vault shares Junior must send to Senior
    function absorbLoss(uint256 vaultShares) external;
}
