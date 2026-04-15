// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {IMoonwellMarket} from "../src/IMoonwellMarket.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";

/// @dev Minimal mock USDC (6 decimals) deployed when USDC_ADDRESS env is unset.
///      Only used on testnets for convenience.
contract MockUSDCDeploy {
    string public name = "USD Coin";
    string public symbol = "USDC";
    uint8 public decimals = 6;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "ERC20: insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(balanceOf[from] >= amount, "ERC20: insufficient balance");
        require(allowance[from][msg.sender] >= amount, "ERC20: insufficient allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

/// @title Deploy
/// @notice Foundry deploy script for PacificaCarryVault.
///         Reads config from env. If USDC_ADDRESS is not set, deploys a mock USDC.
///         After deployment, calls a Node.js helper via FFI to write
///         shared/contracts.json atomically with the full vault ABI.
contract Deploy is Script {
    /// @dev Phase-1 Dol launch cooldown: 30 minutes. Short enough for the
    ///      Scheduled UX to feel meaningful, long enough to demonstrate the
    ///      two-step flow during live demos.
    uint256 constant COOLDOWN_SECONDS = 1800;

    function run() external {
        // ── Read env ────────────────────────────────────────────────────
        uint256 deployerPk = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address operatorAddr = vm.envAddress("OPERATOR_ADDRESS");
        address guardianAddr = vm.envAddress("GUARDIAN_ADDRESS");

        // USDC_ADDRESS and TREASURY_ADDRESS are optional — deploy mocks if unset.
        // For Phase 1 redeploy we MUST reuse the existing USDC + treasury so
        // existing user balances and treasury state survive.
        address usdcAddr = vm.envOr("USDC_ADDRESS", address(0));
        address treasuryAddr = vm.envOr("TREASURY_ADDRESS", address(0));
        // feeRecipient defaults to guardian if FEE_RECIPIENT env is unset.
        address feeRecipientAddr = vm.envOr("FEE_RECIPIENT", guardianAddr);

        vm.startBroadcast(deployerPk);

        // ── Deploy mock USDC if needed ──────────────────────────────────
        if (usdcAddr == address(0)) {
            console.log("USDC_ADDRESS not set, deploying MockUSDC...");
            MockUSDCDeploy mock = new MockUSDCDeploy();
            usdcAddr = address(mock);
            console.log("MockUSDC deployed at:", usdcAddr);
        } else {
            console.log("Reusing existing USDC at:", usdcAddr);
        }

        // ── Reuse or deploy treasury (mock Moonwell market) ─────────────
        IMoonwellMarket treasury;
        if (treasuryAddr == address(0)) {
            MockMoonwellMarket fresh = new MockMoonwellMarket(IERC20(usdcAddr));
            treasury = IMoonwellMarket(address(fresh));
            console.log("MockMoonwellMarket deployed at:", address(treasury));
        } else {
            treasury = IMoonwellMarket(treasuryAddr);
            console.log("Reusing existing treasury at:", treasuryAddr);
        }

        // ── Deploy vault ────────────────────────────────────────────────
        PacificaCarryVault vault = new PacificaCarryVault(
            IERC20(usdcAddr),
            treasury,
            operatorAddr,
            guardianAddr,
            COOLDOWN_SECONDS,
            feeRecipientAddr
        );

        vm.stopBroadcast();

        console.log("=== Deployment Summary ===");
        console.log("Vault:        ", address(vault));
        console.log("USDC:         ", usdcAddr);
        console.log("TreasuryVault:", address(treasury));
        console.log("Operator:     ", operatorAddr);
        console.log("Guardian:     ", guardianAddr);
        console.log("FeeRecipient: ", feeRecipientAddr);
        console.log("Cooldown:     ", COOLDOWN_SECONDS, "seconds");
        console.log("Allocation:    70% margin / 30% treasury");
        console.log("Chain ID:     ", block.chainid);

        // ── Write shared/contracts.json via Node.js helper ──────────────
        _writeContractsJson(
            address(vault),
            usdcAddr,
            address(treasury),
            vm.addr(deployerPk),
            operatorAddr,
            guardianAddr
        );
    }

    /// @dev Calls script/write-contracts-json.js via FFI to atomically write
    ///      shared/contracts.json with the full ABI from the compiled artifact.
    function _writeContractsJson(
        address vault,
        address usdc,
        address treasuryVault,
        address deployer,
        address operatorAddr,
        address guardianAddr
    ) internal {
        string[] memory cmd = new string[](10);
        cmd[0] = "node";
        cmd[1] = "script/write-contracts-json.js";
        cmd[2] = vm.toString(block.chainid);
        cmd[3] = vm.toString(vault);
        cmd[4] = vm.toString(usdc);
        cmd[5] = vm.toString(treasuryVault);
        cmd[6] = vm.toString(deployer);
        cmd[7] = vm.toString(operatorAddr);
        cmd[8] = vm.toString(guardianAddr);
        cmd[9] = vm.toString(block.timestamp);

        bytes memory result = vm.ffi(cmd);
        console.log("contracts.json:", string(result));
    }
}
