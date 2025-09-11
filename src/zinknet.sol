// SPDX-License-Identifier: MIT
pragma solidity ^0.8;

import "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";

contract ZinKNetContract {
    using EnumerableSet for EnumerableSet.AddressSet;

    uint256 public verificationGas = 0.1 ether;
    uint256 public minStake = 0.1 ether;

    struct Task {
        address requestor;
        bytes32 taskHash;
        uint256 reward;
        address prover;
        TaskStatus status;
    }
    enum TaskStatus {
        Open,
        Verifying,
        Completed
    }
    mapping(uint256 => Task) public tasks;
    uint256 private _taskIdCounter;

    mapping(address => uint256) public activeTask;
    mapping(uint256 => EnumerableSet.AddressSet) private _taskDeclarations;

    event TaskCreated(uint256 indexed taskId, address indexed requestor, bytes32 indexed taskHash, uint256 reward);
    event WorkDeclared(uint256 indexed taskId, address indexed prover);

    function getDeclarationCount(uint256 taskId) external view returns (uint256) {
        return _taskDeclarations[taskId].length();
    }

    function getDeclarations(uint256 taskId) external view returns (address[] memory) {
        return _taskDeclarations[taskId].values();
    }

    function createTask(bytes32 taskHash, uint256 reward) external payable {
        require(msg.value == reward + verificationGas, "Incorrect ETH sent");
        require(reward > 0, "Reward must be greater than zero");

        _taskIdCounter++;
        tasks[_taskIdCounter] = Task({
            requestor: msg.sender,
            taskHash: taskHash,
            reward: reward,
            prover: address(0),
            status: TaskStatus.Open
        });
        emit TaskCreated(_taskIdCounter, msg.sender, taskHash, reward);
    }

    function declareWork(uint256 taskId) external payable {
        Task storage task = tasks[taskId];
        require(task.status == TaskStatus.Open, "Task not open");
        require(msg.value == minStake, "Incorrect stake amount");
        require(activeTask[msg.sender] == 0, "Already active in a task");

        _taskDeclarations[taskId].add(msg.sender);
        activeTask[msg.sender] = taskId;
        emit WorkDeclared(taskId, msg.sender);
    }

    function cancelDeclaration(uint256 taskId) external {
        require(activeTask[msg.sender] == taskId, "Not declared for this task");
        Task storage task = tasks[taskId];
        require(task.status == TaskStatus.Open, "Task not open");

        _taskDeclarations[taskId].remove(msg.sender);
        activeTask[msg.sender] = 0;
        (bool sent, ) = msg.sender.call{value: minStake}("");
        require(sent, "ETH transfer failed");
    }
}
