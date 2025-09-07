// SPDX-License-Identifier: MIT
pragma solidity ^0.8;

import "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

contract ZinKNetContract is ReentrancyGuard, EIP712 {
    using ECDSA for bytes32;

    uint256 private _taskIdCounter;

    // ETH関連
    mapping(address => uint256) public stakes;
    uint256 public verificationFee = 0.01 ether;
    uint256 public minStake = 0.1 ether;

    // Task関連
    enum TaskStatus {
        Pending,
        InProgress,
        Completed
    }
    struct Task {
        address requestor;
        address prover;
        uint256 maxPayment;
        uint256 finalPayment;
        TaskStatus status;
    }
    mapping(uint256 => Task) public tasks;

    // EIP-712関連
    bytes32 private constant BID_TYPEHASH =
        keccak256("Bid(uint256 taskId,uint256 bidAmount,address bidder)");

    constructor() EIP712("ZinK Network", "1") {}

    // Events
    event StakeDeposited(address indexed user, uint256 amount);
    event StakeWithdrawn(address indexed user, uint256 amount);
    event TaskCreated(
        uint256 indexed taskId,
        address indexed requestor,
        uint256 maxPayment
    );
    event ProverAssigned(
        uint256 indexed taskId,
        address indexed prover,
        uint256 finalPayment
    );

    // Errors
    error NotTaskRequestor(address caller, address requestor);
    error InsufficientStake(address user, uint256 available, uint256 required);
    error ProverInsufficientStake(
        address prover,
        uint256 proverStake,
        uint256 requiredMinStake
    );
    error InsufficientPayment(uint256 sent, uint256 required);
    error InvalidMaxPayment(uint256 provided);
    error InvalidBidSignature(address recoveredSigner, address expectedProver);
    error TaskNotPending(uint256 taskId, uint8 currentStatus);
    error AddressZero();
    error EtherTransferFailed(address to, uint256 amount);

    // Functions
    function depositToStakes() external payable {
        stakes[msg.sender] += msg.value;
        emit StakeDeposited(msg.sender, msg.value);
    }

    function withdrawFromStakes(uint256 amount) external nonReentrant {
        uint256 available = stakes[msg.sender];
        if (amount > available)
            revert InsufficientStake(msg.sender, available, amount);
        stakes[msg.sender] = available - amount;
        (bool sent, ) = msg.sender.call{value: amount}("");
        if (!sent) revert EtherTransferFailed(msg.sender, amount);
        emit StakeWithdrawn(msg.sender, amount);
    }

    function createTask(uint256 maxPayment) external payable returns (uint256) {
        if (maxPayment == 0) revert InvalidMaxPayment(maxPayment);
        uint256 required = maxPayment + verificationFee;
        if (msg.value != required)
            revert InsufficientPayment(msg.value, required);

        _taskIdCounter++;
        uint256 taskId = _taskIdCounter;
        tasks[taskId] = Task({
            requestor: msg.sender,
            prover: address(0),
            maxPayment: maxPayment,
            finalPayment: 0,
            status: TaskStatus.Pending
        });
        emit TaskCreated(taskId, msg.sender, maxPayment);
        return taskId;
    }

    function _recoverBidSigner(
        uint256 taskId,
        uint256 finalPayment,
        address prover,
        bytes calldata signature
    ) internal view returns (address) {
        bytes32 structHash = keccak256(
            abi.encode(BID_TYPEHASH, taskId, finalPayment, prover)
        );
        bytes32 digest = _hashTypedDataV4(structHash);
        return digest.recover(signature);
    }

    modifier onlyTaskRequestor(uint256 taskId) {
        Task storage t = tasks[taskId];
        if (t.requestor != msg.sender)
            revert NotTaskRequestor(msg.sender, t.requestor);
        _;
    }

    function assignProver(
        uint256 taskId,
        address prover,
        uint256 finalPayment,
        bytes calldata signature
    ) external onlyTaskRequestor(taskId) {
        Task storage task = tasks[taskId];

        if (task.status != TaskStatus.Pending) {
            revert TaskNotPending(taskId, uint8(task.status));
        }
        if (prover == address(0)) revert AddressZero();

        uint256 proverStake = stakes[prover];
        if (proverStake < minStake)
            revert ProverInsufficientStake(prover, proverStake, minStake);

        if (finalPayment == 0) revert InsufficientPayment(finalPayment, 1);
        if (task.maxPayment < finalPayment)
            revert InsufficientPayment(finalPayment, task.maxPayment);

        address recovered = _recoverBidSigner(
            taskId,
            finalPayment,
            prover,
            signature
        );
        if (recovered != prover) revert InvalidBidSignature(recovered, prover);

        task.prover = prover;
        task.status = TaskStatus.InProgress;
        task.finalPayment = finalPayment;
        emit ProverAssigned(taskId, prover, finalPayment);
    }
}
