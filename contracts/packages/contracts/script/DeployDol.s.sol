// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {Dol} from "../src/Dol.sol";

/// @title DeployDol
/// @notice Deploys the Dol token on top of an existing PacificaCarryVault.
///         Phase 1: Dol-only product (Junior tranche deactivated — juniorContract
///         intentionally left unset, so distributeYield() reverts with
///         JuniorNotSet). Reads VAULT_ADDRESS and USDC_ADDRESS from env.
contract DeployDol is Script {
    function run() external {
        address vaultAddr = vm.envAddress("VAULT_ADDRESS");
        address usdcAddr = vm.envAddress("USDC_ADDRESS");
        uint256 deployerPk = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address deployerAddr = vm.addr(deployerPk);

        console.log("=== Dol Deployment ===");
        console.log("Vault:             ", vaultAddr);
        console.log("USDC:              ", usdcAddr);
        console.log("Deployer/Guardian: ", deployerAddr);

        vm.startBroadcast(deployerPk);

        PacificaCarryVault vault = PacificaCarryVault(vaultAddr);

        Dol dol = new Dol(vault, IERC20(usdcAddr), deployerAddr);
        console.log("Dol deployed:       ", address(dol));
        console.log("Junior linkage:    DEACTIVATED (Phase 1 Dol-only product)");

        vm.stopBroadcast();

        _writeContractsJson(vaultAddr, usdcAddr, address(dol));
    }

    function _writeContractsJson(
        address vaultAddr,
        address usdcAddr,
        address dolAddr
    ) internal {
        string[] memory cmd = new string[](6);
        cmd[0] = "node";
        cmd[1] = "script/write-dol-json.js";
        cmd[2] = vm.toString(vaultAddr);
        cmd[3] = vm.toString(usdcAddr);
        cmd[4] = vm.toString(dolAddr);
        cmd[5] = vm.toString(block.number);
        vm.ffi(cmd);
        console.log("shared/contracts.json updated with new Dol address.");
    }
}
