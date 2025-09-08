// SPDX-License-Identifier: MIT
pragma solidity ^0.8;

import {Test, console} from "forge-std/Test.sol";
import {ZinKNetContract} from "../src/zinknet.sol";
import {Vm} from "forge-std/Vm.sol";

// A helper contract to specifically test the ReentrancyGuard
contract GuardTester {
    ZinKNetContract zinknet;

    constructor(ZinKNetContract _zinknet) {
        zinknet = _zinknet;
    }

    function attack(uint256 amount) external {
        zinknet.withdrawFromStakes(amount);
    }

    // This receive hook attempts a re-entrant call that is only blocked
    // by the ReentrancyGuard itself, allowing for a precise test.
    receive() external payable {
        zinknet.withdrawFromStakes(0);
    }
}

contract ZinKNetTest is Test {
    ZinKNetContract internal zinknet;
    uint256 internal constant VERIFICATION_FEE = 0.01 ether;
    uint256 internal constant MIN_STAKE = 0.1 ether;

    address internal requestor;
    address internal prover;
    uint256 internal proverPk;

    // Events and Errors from ZinKNetContract for testing purposes
    event StakeDeposited(address indexed user, uint256 amount);
    event StakeWithdrawn(address indexed user, uint256 amount);
    event TaskCreated(uint256 indexed taskId, address indexed requestor, uint256 maxPayment);
    event ProverAssigned(uint256 indexed taskId, address indexed prover, uint256 finalPayment);

    error NotTaskRequestor(address caller, address requestor);
    error InsufficientStake(address user, uint256 available, uint256 required);
    error ProverInsufficientStake(address prover, uint256 proverStake, uint256 requiredMinStake);
    error InsufficientPayment(uint256 sent, uint256 required);
    error InvalidMaxPayment(uint256 provided);
    error InvalidBidSignature(address recoveredSigner, address expectedProver);
    error TaskNotPending(uint256 taskId, uint8 currentStatus);
    error AddressZero();

    error EtherTransferFailed(address to, uint256 amount);

    function setUp() public {
        zinknet = new ZinKNetContract();
        (prover, proverPk) = makeAddrAndKey("prover");
        requestor = makeAddr("requestor");

        // Fund the test users
        vm.deal(requestor, 10 ether);
        vm.deal(prover, 10 ether);
    }

    /*´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*
     *
     *             STAKING TESTS
     *
     *´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*/

    function test_depositToStakes_succeedsAndUpdatesBalance() public {
        uint256 depositAmount = 1 ether;

        vm.expectEmit(true, false, false, true);
        emit StakeDeposited(requestor, depositAmount);

        vm.prank(requestor);
        zinknet.depositToStakes{value: depositAmount}();

        assertEq(zinknet.stakes(requestor), depositAmount, "Stake should be updated");
    }

    function test_withdrawFromStakes_succeedsAndUpdatesBalance() public {
        uint256 depositAmount = 2 ether;
        uint256 withdrawAmount = 1 ether;

        vm.prank(requestor);
        zinknet.depositToStakes{value: depositAmount}();

        uint256 balanceBefore = requestor.balance;

        vm.expectEmit(true, false, false, true);
        emit StakeWithdrawn(requestor, withdrawAmount);

        vm.prank(requestor);
        zinknet.withdrawFromStakes(withdrawAmount);

        assertEq(zinknet.stakes(requestor), depositAmount - withdrawAmount, "Stake should be reduced");
        assertEq(requestor.balance, balanceBefore + withdrawAmount, "User ETH balance should increase");
    }

    function test_Revert_withdrawFromStakes_whenAmountExceedsStake() public {
        uint256 stakedAmount = 1 ether;
        uint256 withdrawAmount = stakedAmount + 1;

        vm.prank(requestor);
        zinknet.depositToStakes{value: stakedAmount}();

        vm.expectRevert(abi.encodeWithSelector(InsufficientStake.selector, requestor, stakedAmount, withdrawAmount));

        vm.prank(requestor);
        zinknet.withdrawFromStakes(withdrawAmount);
    }

    function test_Revert_withdrawFromStakes_onReentrancy() public {
        // This test uses a specific helper contract (GuardTester) to isolate the nonReentrant guard.
        // The attacker re-enters by calling `withdrawFromStakes(0)`.
        // This inner call is blocked by the ReentrancyGuard, which causes the outer .call to fail.
        // As a result, the final error seen by the test is `EtherTransferFailed`.
        GuardTester attacker = new GuardTester(zinknet);
        uint256 attackerStake = 1 ether;

        // Attacker stakes funds
        vm.deal(address(attacker), attackerStake);
        vm.prank(address(attacker));
        zinknet.depositToStakes{value: attackerStake}();

        // Expect the final EtherTransferFailed error.
        vm.expectRevert(
            abi.encodeWithSelector(EtherTransferFailed.selector, address(attacker), attackerStake)
        );

        // Start the attack by withdrawing the full stake
        attacker.attack(attackerStake);
    }

    /*´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*
     *
     *             TASK CREATION TESTS
     *
     *´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*/

    function test_createTask_succeedsWithCorrectPayment() public {
        uint256 maxPayment = 1 ether;
        uint256 totalPayment = maxPayment + VERIFICATION_FEE;

        vm.expectEmit(true, true, false, true);
        emit TaskCreated(1, requestor, maxPayment);

        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: totalPayment}(maxPayment);

        assertEq(taskId, 1, "Task ID should be 1");
        (
            address requestor_,
            ,
            uint256 maxPayment_,
            ,
            ZinKNetContract.TaskStatus status_
        ) = zinknet.tasks(taskId);
        assertEq(requestor_, requestor);
        assertEq(maxPayment_, maxPayment);
        assertEq(uint(status_), uint(ZinKNetContract.TaskStatus.Pending));
    }

    function test_Revert_createTask_whenPaymentIsIncorrect() public {
        uint256 maxPayment = 1 ether;
        uint256 incorrectPayment = maxPayment; // Missing verification fee

        vm.expectRevert(
            abi.encodeWithSelector(InsufficientPayment.selector, incorrectPayment, maxPayment + VERIFICATION_FEE)
        );

        vm.prank(requestor);
        zinknet.createTask{value: incorrectPayment}(maxPayment);
    }

    function test_Revert_createTask_whenMaxPaymentIsZero() public {
        uint256 maxPayment = 0;
        vm.expectRevert(abi.encodeWithSelector(InvalidMaxPayment.selector, maxPayment));
        vm.prank(requestor);
        zinknet.createTask{value: VERIFICATION_FEE}(maxPayment);
    }

    /*´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*
     *
     *             PROVER ASSIGNMENT TESTS
     *
     *´:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´* `*:°;*°:´*/

    function _createBidSignature(uint256 taskId, uint256 finalPayment, address bidder, uint256 bidderPk)
        internal
        view
        returns (bytes memory)
    {
        // Manually reconstruct the domain separator, mimicking OZ v5.x internal logic
        bytes32 EIP712_DOMAIN_TYPEHASH = keccak256(
            "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
        );
        bytes32 domainSeparator = keccak256(
            abi.encode(
                EIP712_DOMAIN_TYPEHASH,
                keccak256(bytes("ZinK Network")),
                keccak256(bytes("1")),
                block.chainid,
                address(zinknet)
            )
        );

        // Hash the struct data
        bytes32 typehash = keccak256("Bid(uint256 taskId,uint256 bidAmount,address bidder)");
        bytes32 structHash = keccak256(abi.encode(typehash, taskId, finalPayment, bidder));

        // Combine domain separator and struct hash to get the final digest
        bytes32 digest = keccak256(abi.encodePacked(bytes2(0x1901), domainSeparator, structHash));

        // Sign the digest
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(bidderPk, digest);
        return abi.encodePacked(r, s, v);
    }

    function test_assignProver_succeedsWithValidProverAndSignature() public {
        // Arrange: Create a task and have a prover with enough stake
        vm.prank(prover);
        zinknet.depositToStakes{value: MIN_STAKE}();

        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);

        uint256 finalPayment = maxPayment - 0.1 ether;
        bytes memory signature = _createBidSignature(taskId, finalPayment, prover, proverPk);

        // Act & Assert
        vm.expectEmit(true, true, false, true);
        emit ProverAssigned(taskId, prover, finalPayment);

        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);

        (
            ,
            address prover_,
            ,
            uint256 finalPayment_,
            ZinKNetContract.TaskStatus status_
        ) = zinknet.tasks(taskId);
        assertEq(prover_, prover);
        assertEq(finalPayment_, finalPayment);
        assertEq(uint(status_), uint(ZinKNetContract.TaskStatus.InProgress));
    }

    function test_Revert_assignProver_whenSignatureIsInvalid() public {
        vm.prank(prover);
        zinknet.depositToStakes{value: MIN_STAKE}();
        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        uint256 finalPayment = maxPayment;

        // Create a signature for the correct data payload (containing `prover`)
        // but sign it with the WRONG private key (`wrongSignerPk`).
        (address wrongSigner, uint256 wrongSignerPk) = makeAddrAndKey("wrongSigner");
        bytes memory badSig = _createBidSignature(taskId, finalPayment, prover, wrongSignerPk);

        // The contract will recover `wrongSigner` from the signature and see it doesn't match `prover`.
        vm.expectRevert(abi.encodeWithSelector(InvalidBidSignature.selector, wrongSigner, prover));
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, badSig);
    }

    function test_Revert_assignProver_whenCallerIsNotRequestor() public {
        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        bytes memory dummySig;

        vm.expectRevert(abi.encodeWithSelector(NotTaskRequestor.selector, prover, requestor));
        vm.prank(prover); // Wrong caller
        zinknet.assignProver(taskId, prover, maxPayment, dummySig);
    }

    function test_Revert_assignProver_whenTaskIsNotPending() public {
        // Arrange: Create and assign a task
        vm.prank(prover);
        zinknet.depositToStakes{value: MIN_STAKE}();
        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        uint256 finalPayment = maxPayment;
        bytes memory signature = _createBidSignature(taskId, finalPayment, prover, proverPk);
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);

        // Act & Assert: Try to assign again
        vm.expectRevert(abi.encodeWithSelector(TaskNotPending.selector, taskId, uint8(ZinKNetContract.TaskStatus.InProgress)));
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);
    }

    function test_Revert_assignProver_whenProverHasInsufficientStake() public {
        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        uint256 finalPayment = maxPayment;
        bytes memory signature = _createBidSignature(taskId, finalPayment, prover, proverPk);

        // Prover has 0 stake, which is less than MIN_STAKE
        vm.expectRevert(abi.encodeWithSelector(ProverInsufficientStake.selector, prover, 0, MIN_STAKE));
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);
    }

    function test_Revert_assignProver_whenProverIsAddressZero() public {
        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        bytes memory dummySig;

        vm.expectRevert(abi.encodeWithSelector(AddressZero.selector));
        vm.prank(requestor);
        zinknet.assignProver(taskId, address(0), maxPayment, dummySig);
    }

    function test_Revert_assignProver_whenFinalPaymentIsZero() public {
        // Arrange: Prover must have stake for this check to be reached.
        vm.prank(prover);
        zinknet.depositToStakes{value: MIN_STAKE}();

        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        uint256 finalPayment = 0;
        bytes memory signature = _createBidSignature(taskId, finalPayment, prover, proverPk);

        // Act & Assert
        vm.expectRevert(abi.encodeWithSelector(InsufficientPayment.selector, finalPayment, 1));
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);
    }

    function test_Revert_assignProver_whenFinalPaymentExceedsMaxPayment() public {
        // Arrange: Prover must have stake for this check to be reached.
        vm.prank(prover);
        zinknet.depositToStakes{value: MIN_STAKE}();

        uint256 maxPayment = 1 ether;
        vm.prank(requestor);
        uint256 taskId = zinknet.createTask{value: maxPayment + VERIFICATION_FEE}(maxPayment);
        uint256 finalPayment = maxPayment + 1;
        bytes memory signature = _createBidSignature(taskId, finalPayment, prover, proverPk);

        // Act & Assert
        vm.expectRevert(abi.encodeWithSelector(InsufficientPayment.selector, finalPayment, maxPayment));
        vm.prank(requestor);
        zinknet.assignProver(taskId, prover, finalPayment, signature);
    }
}
