// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {PacificaCarryVault} from "../src/PacificaCarryVault.sol";
import {MockMoonwellMarket} from "../src/MockMoonwellMarket.sol";
import {IMoonwellMarket} from "../src/IMoonwellMarket.sol";

/// @dev Minimal mock USDC (6 decimals) for testing.
contract MockUSDC is ERC20 {
    constructor() ERC20("USD Coin", "USDC") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

/// @dev Failing Moonwell market with toggleable failure flags. Used to
///      exercise the vault's mint and redeem failure-branch reverts.
contract FailingMoonwellMarket {
    IERC20 public immutable underlying;
    mapping(address => uint256) public principal;
    bool public failMint;
    bool public failRedeem;

    constructor(IERC20 _underlying) {
        underlying = _underlying;
    }

    function setFailMint(bool v) external { failMint = v; }
    function setFailRedeem(bool v) external { failRedeem = v; }

    function mint(uint256 amount) external returns (uint256) {
        if (failMint) return 7;
        principal[msg.sender] += amount;
        underlying.transferFrom(msg.sender, address(this), amount);
        return 0;
    }

    function redeem(uint256 amount) external returns (uint256) {
        if (failRedeem) return 9;
        principal[msg.sender] -= amount;
        underlying.transfer(msg.sender, amount);
        return 0;
    }

    function balanceOfUnderlying(address acc) external view returns (uint256) {
        return principal[acc];
    }

    function exchangeRateStored() external pure returns (uint256) {
        return 1e18;
    }
}

/// @dev Helper that tries to re-enter the vault on receiving USDC.
contract ReentrantReceiver {
    PacificaCarryVault public vault;
    bool public attacking;

    constructor(PacificaCarryVault _vault) {
        vault = _vault;
    }

    /// @dev Attempt reentrancy on claimWithdraw when we receive USDC.
    ///      This won't actually trigger via standard ERC20 transfer (no
    ///      receive hook), but we expose a manual method to test the guard.
    function attackClaimWithdraw(uint256 requestId) external {
        attacking = true;
        vault.claimWithdraw(requestId);
    }

    function attackDeposit(uint256 assets) external {
        attacking = true;
        vault.deposit(assets, address(this));
    }
}

contract PacificaCarryVaultTest is Test {
    MockUSDC usdc;
    MockMoonwellMarket treasury;
    PacificaCarryVault vault;

    // Use a known private key for operator so we can sign NAV reports
    uint256 constant OPERATOR_PK = 0xA11CE;
    address operator;
    address guardian = makeAddr("guardian");
    address alice    = makeAddr("alice");
    address bob      = makeAddr("bob");

    uint256 constant COOLDOWN = 86400; // 24 hours
    uint256 constant INITIAL_BALANCE = 100_000e6; // 100k USDC

    function setUp() public {
        operator = vm.addr(OPERATOR_PK);

        usdc = new MockUSDC();
        treasury = new MockMoonwellMarket(IERC20(address(usdc)));

        // Pre-fund the treasury so it can pay accrued interest in tests
        usdc.mint(address(treasury), 10_000_000e6);

        vault = new PacificaCarryVault(
            IERC20(address(usdc)),
            treasury,
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        // Fund test users
        usdc.mint(alice, INITIAL_BALANCE);
        usdc.mint(bob, INITIAL_BALANCE);

        // Approve vault
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(vault), type(uint256).max);
    }

    // ═══════════════════════════════════════════════════════════════════
    // DEPOSIT TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Deposit happy path: user deposits USDC, receives shares,
    ///         vault balance increases.
    /// Protects: basic deposit accounting — shares minted == assets
    ///           deposited at 1:1 for first deposit.
    function test_deposit_happyPath() public {
        uint256 assets = 1000e6;

        vm.prank(alice);
        uint256 shares = vault.deposit(assets, alice);

        // 30% of the deposit is routed to the treasury, 70% stays as idle USDC.
        uint256 expectedTreasury = (assets * 3000) / 10000;
        uint256 expectedIdle = assets - expectedTreasury;

        assertEq(shares, assets, "first deposit should be 1:1");
        assertEq(vault.balanceOf(alice), shares, "alice should hold shares");
        assertEq(vault.totalAssets(), assets, "totalAssets should reflect full deposit");
        assertEq(usdc.balanceOf(address(vault)), expectedIdle, "vault holds 70% as idle USDC");
        assertEq(
            treasury.balanceOfUnderlying(address(vault)),
            expectedTreasury,
            "vault holds 30% in treasury"
        );
    }

    /// @notice Deposit when paused must revert with VaultPaused.
    /// Protects: pause mechanism blocks all deposits.
    function test_deposit_whenPaused_reverts() public {
        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.deposit(1000e6, alice);
    }

    /// @notice Deposit with zero assets must revert with ZeroAssets.
    /// Protects: prevents no-op deposits that could pollute accounting.
    function test_deposit_zeroAssets_reverts() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroAssets.selector);
        vault.deposit(0, alice);
    }

    /// @notice First deposit initializes share price at 1:1.
    /// Protects: correct initial share price — no inflation attack on
    ///           empty vault.
    function test_firstDeposit_sharePriceInit() public {
        vm.prank(alice);
        vault.deposit(5000e6, alice);

        // share price should be 1e18 (1:1 scaled)
        assertEq(vault.sharePrice(), 1e18, "initial share price should be 1e18");
    }

    /// @notice Subsequent deposits use the correct share price math.
    /// Protects: share price consistency across multiple deposits —
    ///           second depositor gets fair shares relative to first.
    function test_subsequentDeposit_sharePriceMath() public {
        // Alice deposits 1000 USDC first
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        // Bob deposits 2000 USDC second
        vm.prank(bob);
        uint256 bobShares = vault.deposit(2000e6, bob);

        // At 1:1 share price, bob should get 2000e6 shares
        assertEq(bobShares, 2000e6, "bob shares should reflect 1:1 price");
        assertEq(vault.totalAssets(), 3000e6, "total assets = 3000 USDC");

        // Share price should remain 1e18
        assertEq(vault.sharePrice(), 1e18, "share price unchanged at 1:1");
    }

    // ═══════════════════════════════════════════════════════════════════
    // REQUEST WITHDRAW TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice requestWithdraw burns shares immediately and emits event.
    /// Protects: shares are removed from circulation at request time,
    ///           preventing double-spend.
    function test_requestWithdraw_burnsShares_emitsEvent() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        vm.expectEmit(true, false, false, true);
        emit PacificaCarryVault.WithdrawRequested(0, alice, 1000e6, 1000e6);
        uint256 requestId = vault.requestWithdraw(1000e6);

        assertEq(requestId, 0, "first request id should be 0");
        assertEq(vault.balanceOf(alice), 0, "shares burned immediately");
    }

    /// @notice requestWithdraw with zero shares must revert.
    /// Protects: prevents empty withdraw requests.
    function test_requestWithdraw_zeroShares_reverts() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroShares.selector);
        vault.requestWithdraw(0);
    }

    /// @notice requestWithdraw with more shares than balance reverts
    ///         (ERC20 burn underflow).
    /// Protects: user cannot withdraw more than deposited.
    function test_requestWithdraw_insufficientShares_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        vm.expectRevert(); // ERC20 burn underflow
        vault.requestWithdraw(2000e6);
    }

    // ═══════════════════════════════════════════════════════════════════
    // CLAIM WITHDRAW TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice claimWithdraw before cooldown must revert.
    /// Protects: 24h cooldown is enforced — no early withdrawals.
    function test_claimWithdraw_beforeCooldown_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        // Try to claim immediately
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.CooldownNotElapsed.selector);
        vault.claimWithdraw(requestId);
    }

    /// @notice claimWithdraw after cooldown — happy path.
    /// Protects: user receives USDC after waiting the full cooldown.
    function test_claimWithdraw_afterCooldown_happyPath() public {
        uint256 depositAmount = 1000e6;

        vm.prank(alice);
        vault.deposit(depositAmount, alice);

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        // Advance time past cooldown
        vm.warp(block.timestamp + COOLDOWN);

        uint256 balanceBefore = usdc.balanceOf(alice);

        vm.prank(alice);
        vm.expectEmit(true, false, false, true);
        emit PacificaCarryVault.WithdrawClaimed(requestId, alice, depositAmount);
        uint256 claimed = vault.claimWithdraw(requestId);

        assertEq(claimed, depositAmount, "claimed amount should match deposit");
        assertEq(
            usdc.balanceOf(alice),
            balanceBefore + depositAmount,
            "alice should receive USDC"
        );
        // After full withdraw, only residual treasury yield remains (claim
        // redeems exactly what is needed, leaving any accrued interest in
        // the treasury for the next user). Idle is fully drained.
        assertEq(usdc.balanceOf(address(vault)), 0, "vault idle should be drained");
        assertEq(vault.totalAssetsStored(), 0, "margin slot should be 0");
    }

    /// @notice claimWithdraw twice — second call must revert.
    /// Protects: claim ticket is single-use — no double-claim.
    function test_claimWithdraw_twice_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(alice);
        vault.claimWithdraw(requestId);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.AlreadyClaimed.selector);
        vault.claimWithdraw(requestId);
    }

    /// @notice claimWithdraw by non-owner must revert.
    /// Protects: only the original requester can claim their ticket.
    function test_claimWithdraw_notOwner_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(bob);
        vm.expectRevert(PacificaCarryVault.NotRequestOwner.selector);
        vault.claimWithdraw(requestId);
    }

    // ═══════════════════════════════════════════════════════════════════
    // DISABLED ERC-4626 FUNCTIONS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Standard ERC-4626 withdraw is disabled.
    /// Protects: forces all withdrawals through the queue.
    function test_withdraw_disabled() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.WithdrawDisabled.selector);
        vault.withdraw(100e6, alice, alice);
    }

    /// @notice Standard ERC-4626 redeem is disabled.
    /// Protects: forces all withdrawals through the queue.
    function test_redeem_disabled() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.WithdrawDisabled.selector);
        vault.redeem(100e6, alice, alice);
    }

    /// @notice Standard ERC-4626 mint is disabled.
    /// Protects: only deposit() is the entry point.
    function test_mint_disabled() public {
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.WithdrawDisabled.selector);
        vault.mint(100e6, alice);
    }

    // ═══════════════════════════════════════════════════════════════════
    // SHARE PRICE
    // ═══════════════════════════════════════════════════════════════════

    /// @notice sharePrice returns 1e18 when vault is empty.
    /// Protects: no division by zero on empty vault.
    function test_sharePrice_emptyVault() public view {
        assertEq(vault.sharePrice(), 1e18, "empty vault share price = 1e18");
    }

    // ═══════════════════════════════════════════════════════════════════
    // NAV REPORTER TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice reportNAV by operator — happy path (first report).
    /// Protects: operator can submit a valid signed NAV update and the
    ///           oracle slot is updated correctly.
    function test_reportNAV_operator_happyPath() public {
        // Seed the vault with a deposit so totalAssetsStored > 0
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 newNav = 1050e6; // 5% gain
        uint256 ts = block.timestamp + 1;

        // First report: skips delta check
        bytes memory sig = _signNav(newNav, ts, OPERATOR_PK);

        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.NavReported(newNav, ts);
        vault.reportNAV(newNav, ts, sig);

        // In V1.5, reportNAV updates the off-chain margin slot, not the full NAV.
        assertEq(vault.totalAssetsStored(), newNav, "margin slot should be updated");
        assertEq(vault.lastTimestamp(), ts, "lastTimestamp should be updated");
        assertTrue(vault.navInitialized(), "navInitialized should be true");
    }

    /// @notice reportNAV by non-operator must revert.
    /// Protects: only the operator key can update NAV — prevents
    ///           unauthorized manipulation of the oracle slot.
    function test_reportNAV_nonOperator_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 newNav = 1050e6;
        uint256 ts = block.timestamp + 1;

        // Sign with a different key (bob's)
        uint256 bobPk = 0xB0B;
        bytes memory sig = _signNav(newNav, ts, bobPk);

        vm.expectRevert(PacificaCarryVault.InvalidNavSignature.selector);
        vault.reportNAV(newNav, ts, sig);
    }

    /// @notice reportNAV with stale timestamp must revert.
    /// Protects: monotonic timestamp ensures NAV reports cannot be
    ///           replayed or submitted out of order.
    function test_reportNAV_staleTimestamp_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 ts1 = block.timestamp + 1;
        uint256 ts2 = ts1; // same timestamp = stale

        // First report succeeds
        vault.reportNAV(1050e6, ts1, _signNav(1050e6, ts1, OPERATOR_PK));

        // Second report with same or earlier timestamp must revert
        vm.expectRevert(PacificaCarryVault.StaleTimestamp.selector);
        vault.reportNAV(1060e6, ts2, _signNav(1060e6, ts2, OPERATOR_PK));
    }

    /// @notice reportNAV with >10% delta must revert.
    /// Protects: sanity guard prevents catastrophic oracle manipulation —
    ///           a single report cannot move NAV by more than 10%.
    function test_reportNAV_deltaOver10Percent_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 ts1 = block.timestamp + 1;
        // First report (skips delta check)
        vault.reportNAV(1000e6, ts1, _signNav(1000e6, ts1, OPERATOR_PK));

        // Second report: +11% = 1110e6 (exceeds 10%)
        uint256 newNav = 1110e6;
        uint256 ts2 = ts1 + 1;

        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2, OPERATOR_PK));
    }

    /// @notice reportNAV with exactly 10% delta must revert (boundary).
    /// Protects: the guard is strict — exactly 10% is rejected because
    ///           the condition is `delta * 10 >= lastNav` (not >).
    function test_reportNAV_deltaExactly10Percent_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(1000e6, ts1, _signNav(1000e6, ts1, OPERATOR_PK));

        // Exactly 10%: 1000 + 100 = 1100 → delta=100, 100*10=1000 >= 1000 → revert
        uint256 newNav = 1100e6;
        uint256 ts2 = ts1 + 1;

        vm.expectRevert(PacificaCarryVault.NavDeltaTooLarge.selector);
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2, OPERATOR_PK));
    }

    /// @notice reportNAV with invalid signature must revert.
    /// Protects: tampered or corrupted signatures are rejected.
    function test_reportNAV_invalidSignature_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 newNav = 1050e6;
        uint256 ts = block.timestamp + 1;

        // Sign correct payload but corrupt the signature bytes
        bytes memory sig = _signNav(newNav, ts, OPERATOR_PK);
        sig[0] = bytes1(uint8(sig[0]) ^ 0xFF); // flip first byte

        vm.expectRevert(); // ECDSA recover will revert or return wrong address
        vault.reportNAV(newNav, ts, sig);
    }

    /// @notice reportNAV within 10% succeeds (just under boundary).
    /// Protects: valid NAV changes within the guard pass correctly.
    function test_reportNAV_deltaJustUnder10Percent_succeeds() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(1000e6, ts1, _signNav(1000e6, ts1, OPERATOR_PK));

        // 9.9%: 1000 + 99 = 1099 → delta=99, 99*10=990 < 1000 → passes
        uint256 newNav = 1099e6;
        uint256 ts2 = ts1 + 1;
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2, OPERATOR_PK));

        assertEq(vault.totalAssetsStored(), newNav, "margin slot updated to 1099e6");
    }

    /// @notice sharePrice reflects NAV changes after reportNAV.
    /// Protects: share price correctly tracks total NAV (idle + treasury + margin slot).
    function test_sharePrice_afterNavReport() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        // Total NAV = 1000 (700 idle + 300 treasury + 0 stored).
        // Report 50e6 in the off-chain margin slot — this represents PnL
        // from the perp position. New total = 1000 + 50 = 1050. +5%.
        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(50e6, ts1, _signNav(50e6, ts1, OPERATOR_PK));

        // sharePrice = 1050e6 * 1e18 / 1000e6 = 1.05e18
        assertEq(vault.sharePrice(), 1.05e18, "share price should reflect 5% gain");
    }

    /// @notice reportNAV with negative delta within 10% succeeds.
    /// Protects: NAV can decrease (loss scenario) as long as within guard.
    function test_reportNAV_negativeDelta_succeeds() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(1000e6, ts1, _signNav(1000e6, ts1, OPERATOR_PK));

        // -5%: 1000 - 50 = 950 → delta=50, 50*10=500 < 1000 → passes
        uint256 newNav = 950e6;
        uint256 ts2 = ts1 + 1;
        vault.reportNAV(newNav, ts2, _signNav(newNav, ts2, OPERATOR_PK));

        assertEq(vault.totalAssetsStored(), newNav, "margin slot updated to 950e6");
    }

    // ═══════════════════════════════════════════════════════════════════
    // ACCESS CONTROL + PAUSE TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice Guardian can pause the vault.
    /// Protects: guardian key can halt deposits and claims in emergencies.
    function test_pause_byGuardian_happyPath() public {
        vm.prank(guardian);
        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.Paused(guardian);
        vault.pause();

        assertTrue(vault.paused(), "vault should be paused");
    }

    /// @notice Non-guardian cannot pause the vault.
    /// Protects: only GUARDIAN_ROLE can trigger an emergency pause.
    function test_pause_byNonGuardian_reverts() public {
        vm.prank(alice);
        vm.expectRevert();
        vault.pause();
    }

    /// @notice Guardian can unpause the vault.
    /// Protects: guardian can resume normal operations after emergency.
    function test_unpause_byGuardian_happyPath() public {
        vm.prank(guardian);
        vault.pause();

        vm.prank(guardian);
        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.Unpaused(guardian);
        vault.unpause();

        assertFalse(vault.paused(), "vault should be unpaused");
    }

    /// @notice Non-guardian cannot unpause the vault.
    /// Protects: only GUARDIAN_ROLE can lift an emergency pause.
    function test_unpause_byNonGuardian_reverts() public {
        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert();
        vault.unpause();
    }

    /// @notice Deposits are blocked when paused.
    /// Protects: no new funds enter the vault during an emergency.
    function test_deposit_blockedWhenPaused() public {
        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.deposit(1000e6, alice);
    }

    /// @notice claimWithdraw is blocked when paused.
    /// Protects: no funds leave the vault during an emergency.
    function test_claimWithdraw_blockedWhenPaused() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.claimWithdraw(requestId);
    }

    /// @notice requestWithdraw is allowed when paused — users can queue exits.
    /// Protects: users can always signal intent to exit, even during emergency.
    function test_requestWithdraw_allowedWhenPaused() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(1000e6);

        assertEq(requestId, 0, "request should succeed while paused");
        assertEq(vault.balanceOf(alice), 0, "shares burned while paused");
    }

    /// @notice reportNAV is allowed when paused — oracle keeps running.
    /// Protects: NAV updates continue during emergency so share price stays
    ///           accurate when the vault unpauses.
    function test_reportNAV_allowedWhenPaused() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(guardian);
        vault.pause();

        uint256 newNav = 1050e6;
        uint256 ts = block.timestamp + 1;
        bytes memory sig = _signNav(newNav, ts, OPERATOR_PK);

        vault.reportNAV(newNav, ts, sig);
        assertEq(vault.totalAssetsStored(), newNav, "margin slot updated while paused");
    }

    /// @notice Guardian can rotate the operator. New operator can sign NAV.
    /// Protects: key rotation works end-to-end — new key signs, old key rejected.
    function test_setOperator_byGuardian_newOperatorCanSign() public {
        uint256 newOpPk = 0xDEAD;
        address newOp = vm.addr(newOpPk);

        vm.prank(guardian);
        vm.expectEmit(true, true, false, false);
        emit PacificaCarryVault.OperatorChanged(operator, newOp);
        vault.setOperator(newOp);

        assertEq(vault.operator(), newOp, "operator should be updated");

        // New operator can sign NAV reports
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 newNav = 1050e6;
        uint256 ts = block.timestamp + 1;
        bytes memory sig = _signNav(newNav, ts, newOpPk);
        vault.reportNAV(newNav, ts, sig);

        assertEq(vault.totalAssetsStored(), newNav, "margin slot updated by new operator");

        // Old operator's signature is now rejected
        uint256 ts2 = ts + 1;
        bytes memory oldSig = _signNav(1060e6, ts2, OPERATOR_PK);
        vm.expectRevert(PacificaCarryVault.InvalidNavSignature.selector);
        vault.reportNAV(1060e6, ts2, oldSig);
    }

    /// @notice Non-guardian cannot rotate the operator.
    /// Protects: only GUARDIAN_ROLE can change the operator key.
    function test_setOperator_byNonGuardian_reverts() public {
        vm.prank(alice);
        vm.expectRevert();
        vault.setOperator(alice);
    }

    /// @notice Guardian can rotate the guardian. Old guardian loses authority.
    /// Protects: key handoff is clean — old guardian cannot pause after transfer.
    function test_setGuardian_transfersPauseAuthority() public {
        address newGuardian = makeAddr("newGuardian");

        vm.prank(guardian);
        vm.expectEmit(true, true, false, false);
        emit PacificaCarryVault.GuardianChanged(guardian, newGuardian);
        vault.setGuardian(newGuardian);

        assertEq(vault.guardian(), newGuardian, "guardian should be updated");

        // New guardian can pause
        vm.prank(newGuardian);
        vault.pause();
        assertTrue(vault.paused(), "new guardian should be able to pause");

        // Old guardian cannot unpause
        vm.prank(guardian);
        vm.expectRevert();
        vault.unpause();
    }

    /// @notice Non-guardian cannot rotate the guardian.
    /// Protects: only GUARDIAN_ROLE can transfer guardian authority.
    function test_setGuardian_byNonGuardian_reverts() public {
        vm.prank(alice);
        vm.expectRevert();
        vault.setGuardian(alice);
    }

    /// @notice Pausing an already-paused vault reverts.
    /// Protects: prevents redundant pause calls and misleading Paused events.
    function test_pause_whenAlreadyPaused_reverts() public {
        vm.prank(guardian);
        vault.pause();

        vm.prank(guardian);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.pause();
    }

    /// @notice Unpausing a non-paused vault reverts.
    /// Protects: prevents redundant unpause calls and misleading Unpaused events.
    function test_unpause_whenNotPaused_reverts() public {
        vm.prank(guardian);
        vm.expectRevert(PacificaCarryVault.VaultNotPaused.selector);
        vault.unpause();
    }

    /// @notice supportsInterface returns true for AccessControl interface.
    /// Protects: ERC-165 compatibility for on-chain role discovery.
    function test_supportsInterface() public view {
        // AccessControl interface ID
        assertTrue(vault.supportsInterface(type(IAccessControl).interfaceId));
    }

    // ═══════════════════════════════════════════════════════════════════
    // V1.5 TWO-TIER YIELD TESTS
    // ═══════════════════════════════════════════════════════════════════

    /// @notice deposit() splits funds 70/30 between idle USDC and the treasury.
    /// Protects: the V1.5 allocation policy is correctly enforced on every deposit.
    function test_deposit_splitsTreasuryAndMargin() public {
        uint256 amount = 10_000e6;

        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.TreasuryDeposited(3_000e6);
        vm.prank(alice);
        vault.deposit(amount, alice);

        assertEq(usdc.balanceOf(address(vault)), 7_000e6, "70% should remain idle");
        assertEq(
            treasury.balanceOfUnderlying(address(vault)),
            3_000e6,
            "30% should be in treasury"
        );
        assertEq(treasury.principalOf(address(vault)), 3_000e6, "treasury principal recorded");
    }

    /// @notice totalAssets() includes the treasury underlying balance.
    /// Protects: the new totalAssets() formula sums all three buckets correctly.
    function test_totalAssets_includesTreasuryBalance() public {
        vm.prank(alice);
        vault.deposit(10_000e6, alice);

        // 7000 idle + 3000 treasury + 0 stored = 10000
        assertEq(vault.totalAssets(), 10_000e6, "totalAssets should sum all buckets");
    }

    /// @notice Treasury balance grows over time at 5% APY.
    /// Protects: time-based accrual is reflected live in totalAssets() and
    ///           share price, without needing an explicit reportNAV.
    function test_treasuryAccrualOverTime() public {
        vm.prank(alice);
        vault.deposit(10_000e6, alice);

        // Warp 1 year — treasury should grow by 5% on 3000 USDC = 150 USDC
        vm.warp(block.timestamp + 365 days);

        uint256 treasuryBal = treasury.balanceOfUnderlying(address(vault));
        assertEq(treasuryBal, 3_150e6, "treasury should accrue 150 USDC over 1 year");

        // totalAssets reflects the live treasury balance: 7000 + 3150 + 0 = 10150
        assertEq(vault.totalAssets(), 10_150e6, "totalAssets should include treasury gains");

        // Share price reflects the gain: 10150 * 1e18 / 10000 = 1.015e18
        assertEq(vault.sharePrice(), 1.015e18, "share price should reflect treasury yield");
    }

    /// @notice Withdrawing after treasury accrual delivers principal + yield.
    /// Protects: depositors capture the base yield from the treasury layer.
    function test_withdrawWhenTreasuryHasGains() public {
        vm.prank(alice);
        vault.deposit(10_000e6, alice);

        // Accrue 1 year of yield
        vm.warp(block.timestamp + 365 days);

        // alice's shares now represent 10150 USDC of NAV
        uint256 shares = vault.balanceOf(alice);
        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(shares);

        (, uint256 owed, , ) = vault.withdrawRequests(requestId);
        // With 10150 totalAssets and 10000 supply, convertToAssets(10000) ≈ 10150
        assertGe(owed, 10_149e6, "claim should reflect treasury yield");
        assertLe(owed, 10_151e6, "claim should reflect treasury yield");

        vm.warp(block.timestamp + COOLDOWN);

        uint256 balBefore = usdc.balanceOf(alice);
        vm.prank(alice);
        uint256 claimed = vault.claimWithdraw(requestId);

        assertEq(claimed, owed, "claimed amount equals locked amount");
        assertEq(
            usdc.balanceOf(alice),
            balBefore + owed,
            "alice receives principal + treasury yield"
        );
    }

    /// @notice Partial withdraw pulls from idle first, then treasury for the rest.
    /// Protects: claim source ordering — idle is consumed before treasury,
    ///           and treasury redemption is only triggered when idle is insufficient.
    function test_partialWithdrawWithMixedSources() public {
        vm.prank(alice);
        vault.deposit(10_000e6, alice);

        // Idle = 7000, treasury = 3000. Withdraw 80% (8000 shares ≈ 8000 USDC).
        uint256 sharesToBurn = 8_000e6;
        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(sharesToBurn);

        vm.warp(block.timestamp + COOLDOWN);

        // Claim should pull 7000 from idle and ~1000 from treasury.
        vm.expectEmit(false, false, false, true);
        emit PacificaCarryVault.TreasuryRedeemed(1_000e6);
        vm.prank(alice);
        uint256 claimed = vault.claimWithdraw(requestId);

        assertEq(claimed, 8_000e6, "claimed = 8000 USDC");
        assertEq(usdc.balanceOf(address(vault)), 0, "idle should be drained");
        // Treasury retains ~2000 USDC of principal (small amount of accrued
        // yield from the cooldown period stays as residual).
        uint256 treasuryRemaining = treasury.balanceOfUnderlying(address(vault));
        assertGe(treasuryRemaining, 2_000e6, "treasury should retain >= 2000 USDC");
        assertLt(treasuryRemaining, 2_001e6, "treasury residual should be tiny vs principal");
    }

    /// @notice Treasury mint failure (non-zero error code) reverts with TreasuryMintFailed.
    /// Protects: the vault correctly surfaces non-zero return codes from
    ///           the treasury market — no silent failures swallow Compound errors.
    function test_deposit_treasuryMintFails_reverts() public {
        FailingMoonwellMarket failingTreasury = new FailingMoonwellMarket(IERC20(address(usdc)));
        PacificaCarryVault failingVault = new PacificaCarryVault(
            IERC20(address(usdc)),
            IMoonwellMarket(address(failingTreasury)),
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        vm.prank(alice);
        usdc.approve(address(failingVault), type(uint256).max);

        failingTreasury.setFailMint(true);

        vm.prank(alice);
        vm.expectRevert(
            abi.encodeWithSelector(PacificaCarryVault.TreasuryMintFailed.selector, uint256(7))
        );
        failingVault.deposit(1000e6, alice);
    }

    /// @notice Treasury redeem failure (non-zero error code) reverts with TreasuryRedeemFailed.
    /// Protects: the vault surfaces non-zero return codes during a claim
    ///           that needs to redeem from the treasury.
    function test_claimWithdraw_treasuryRedeemFails_reverts() public {
        FailingMoonwellMarket failingTreasury = new FailingMoonwellMarket(IERC20(address(usdc)));
        PacificaCarryVault failingVault = new PacificaCarryVault(
            IERC20(address(usdc)),
            IMoonwellMarket(address(failingTreasury)),
            operator,
            guardian,
            COOLDOWN,
            guardian
        );

        // Normal deposit succeeds — failMint is false initially.
        vm.startPrank(alice);
        usdc.approve(address(failingVault), type(uint256).max);
        failingVault.deposit(1000e6, alice);
        uint256 requestId = failingVault.requestWithdraw(1000e6);
        vm.stopPrank();

        vm.warp(block.timestamp + COOLDOWN);

        // Now flip the redeem failure flag — claim must pull 300 from treasury.
        failingTreasury.setFailRedeem(true);

        vm.prank(alice);
        vm.expectRevert(
            abi.encodeWithSelector(PacificaCarryVault.TreasuryRedeemFailed.selector, uint256(9))
        );
        failingVault.claimWithdraw(requestId);
    }

    /// @notice Tiny deposits where 30% rounds to 0 still succeed and bypass
    ///         the treasury mint call entirely.
    /// Protects: the `if (toTreasury > 0)` branch — when the deposit is so
    ///           small that 30% would be zero (e.g. 3 USDC * 30% = 0 due to
    ///           integer rounding), the vault must skip the mint and not
    ///           revert with ZeroAmount from the treasury.
    function test_deposit_tinyAmount_skipsTreasuryMint() public {
        // 3 wei * 3000 / 10000 = 0 → toTreasury == 0, mint is skipped
        vm.prank(alice);
        uint256 shares = vault.deposit(3, alice);

        assertEq(shares, 3, "shares minted 1:1");
        assertEq(usdc.balanceOf(address(vault)), 3, "all 3 wei stays idle");
        assertEq(treasury.principalOf(address(vault)), 0, "no treasury principal");
    }

    /// @notice Claim reverts cleanly when idle + treasury cannot cover the request.
    /// Protects: the vault never silently underpays — if liquidity is insufficient,
    ///           the claim reverts with InsufficientLiquidity rather than fulfilling
    ///           a partial amount.
    function test_claimWithdraw_insufficientLiquidity_reverts() public {
        vm.prank(alice);
        vault.deposit(10_000e6, alice);

        // Bot reports 50_000 of off-chain margin (without actually staging USDC).
        // Now totalAssets = 7000 + 3000 + 50000 = 60000.
        uint256 ts1 = block.timestamp + 1;
        vault.reportNAV(50_000e6, ts1, _signNav(50_000e6, ts1, OPERATOR_PK));

        // alice's 10000 shares now claim ~60000 USDC, which exceeds idle+treasury (10000).
        uint256 shares = vault.balanceOf(alice);
        vm.prank(alice);
        uint256 requestId = vault.requestWithdraw(shares);

        vm.warp(block.timestamp + COOLDOWN);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.InsufficientLiquidity.selector);
        vault.claimWithdraw(requestId);
    }

    // ═══════════════════════════════════════════════════════════════════
    // HELPERS
    // ═══════════════════════════════════════════════════════════════════

    /// @dev Signs a NAV report using the exact payload from INTERFACES.md §3.
    function _signNav(uint256 newNav, uint256 timestamp, uint256 pk)
        internal
        view
        returns (bytes memory)
    {
        bytes32 payloadHash = keccak256(
            abi.encodePacked(
                "PACIFICA_CARRY_VAULT_NAV",
                address(vault),
                newNav,
                timestamp
            )
        );
        bytes32 ethSignedHash = _toEthSignedMessageHash(payloadHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, ethSignedHash);
        return abi.encodePacked(r, s, v);
    }

    /// @dev Reproduces the EIP-191 personal_sign prefix.
    function _toEthSignedMessageHash(bytes32 hash) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", hash));
    }

    // ─────────────────────────────────────────────────────────────────────
    // instantRedeem — Phase-1 Dol fast path (5 bps fee, idle-USDC only)
    // ─────────────────────────────────────────────────────────────────────

    /// @notice Happy path: instantRedeem burns shares, pays 5 bps fee to
    ///         feeRecipient, transfers net USDC to caller in one tx.
    /// Protects: the fast-lane promise that backs the Dol "Instant" button.
    function test_instantRedeem_happyPath() public {
        // Alice deposits 1000 USDC at share price = 1.0 (first depositor)
        vm.prank(alice);
        uint256 shares = vault.deposit(1000e6, alice);
        assertEq(shares, 1000e6, "shares == assets on first deposit");

        // feeRecipient starts at guardian per constructor
        uint256 guardianUsdcBefore = usdc.balanceOf(guardian);
        uint256 aliceUsdcBefore = usdc.balanceOf(alice);

        // Alice instant-redeems half of her shares (500e6)
        vm.prank(alice);
        uint256 out = vault.instantRedeem(500e6);

        // Gross = 500e6, fee = 500e6 * 5 / 10000 = 250_000 (0.25 USDC)
        uint256 expectedGross = 500e6;
        uint256 expectedFee = (expectedGross * 5) / 10_000;
        uint256 expectedNet = expectedGross - expectedFee;

        assertEq(out, expectedNet, "return value is net amount");
        assertEq(usdc.balanceOf(alice) - aliceUsdcBefore, expectedNet, "alice got net");
        assertEq(usdc.balanceOf(guardian) - guardianUsdcBefore, expectedFee, "fee to recipient");
        assertEq(vault.balanceOf(alice), 500e6, "half shares remain");
    }

    /// @notice instantRedeem reverts when called with zero shares.
    /// Protects: zero-input paths that would otherwise emit a no-op event.
    function test_instantRedeem_zeroShares_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.ZeroShares.selector);
        vault.instantRedeem(0);
    }

    /// @notice instantRedeem reverts when the vault is paused.
    /// Protects: guardian pause as a circuit breaker against all withdrawals.
    function test_instantRedeem_whenPaused_reverts() public {
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        vm.prank(guardian);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.VaultPaused.selector);
        vault.instantRedeem(500e6);
    }

    /// @notice instantRedeem reverts when idle USDC is below the gross
    ///         amount — the fast lane is served from idle only, never from
    ///         the treasury.
    /// Protects: the invariant that Instant never triggers a slow treasury
    ///           redemption. Frontend relies on this revert to route to Scheduled.
    function test_instantRedeem_insufficientLiquidity_reverts() public {
        // Alice deposits 1000 USDC: 700 stays idle, 300 goes to treasury
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        // Try to instantly pull 800 USDC — gross exceeds idle (700)
        // convertToAssets is 1:1 because share price is unchanged.
        vm.prank(alice);
        vm.expectRevert(PacificaCarryVault.InsufficientLiquidity.selector);
        vault.instantRedeem(800e6);
    }

    /// @notice instantRedeem fee math: exact bps computation + rounding.
    /// Protects: the 5 bps fee is computed from gross, not net, and uses
    ///           the BPS_DENOMINATOR constant.
    function test_instantRedeem_feeMath_exact() public {
        // Alice deposits 2_000_000 USDC at 1:1
        usdc.mint(alice, 2_000_000e6);
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(alice);
        vault.deposit(2_000_000e6, alice);

        uint256 beforeFeeBal = usdc.balanceOf(guardian);

        // Redeem 2_000_000e6 shares → gross = 2_000_000e6, fee = 1000e6 (1000 USDC)
        // Requires idle ≥ gross: 70% of 2_000_000e6 + original 700e6 = 1_401_700e6 idle.
        // So redeem shares worth 1_000_000e6 — idle covers it.
        vm.prank(alice);
        uint256 out = vault.instantRedeem(1_000_000e6);
        uint256 expectedFee = (1_000_000e6 * 5) / 10_000; // 500e6 = 500 USDC
        uint256 expectedNet = 1_000_000e6 - expectedFee;

        assertEq(out, expectedNet, "net amount exact");
        assertEq(usdc.balanceOf(guardian) - beforeFeeBal, expectedFee, "fee exact");
    }

    /// @notice setFeeRecipient rotates the destination address and rejects
    ///         zero-address. Only GUARDIAN_ROLE can call.
    /// Protects: the fee-revenue routing controlled by the guardian, with
    ///           guardrails against accidentally burning fees to address(0).
    function test_instantRedeem_feeRecipient_rotation() public {
        address newRecipient = makeAddr("newRecipient");
        bytes32 guardianRole = vault.GUARDIAN_ROLE();

        // Non-guardian cannot rotate
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector,
                alice,
                guardianRole
            )
        );
        vm.prank(alice);
        vault.setFeeRecipient(newRecipient);

        // Guardian cannot set to zero
        vm.prank(guardian);
        vm.expectRevert(PacificaCarryVault.ZeroAddress.selector);
        vault.setFeeRecipient(address(0));

        // Guardian can rotate
        vm.prank(guardian);
        vault.setFeeRecipient(newRecipient);
        assertEq(vault.feeRecipient(), newRecipient, "recipient rotated");

        // Subsequent fee flows to new recipient
        vm.prank(alice);
        vault.deposit(1000e6, alice);

        uint256 before = usdc.balanceOf(newRecipient);
        vm.prank(alice);
        vault.instantRedeem(500e6);
        uint256 expectedFee = (500e6 * 5) / 10_000;
        assertEq(usdc.balanceOf(newRecipient) - before, expectedFee, "new recipient got fee");
    }

}
