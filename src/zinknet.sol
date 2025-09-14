// SPDX-License-Identifier: MIT
pragma solidity ^0.8;

import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";

contract ZinKNetContract {
    using EnumerableSet for EnumerableSet.AddressSet;

    uint256 public verificationFee = 0.1 ether;
    uint256 public minStake = 0.1 ether;

    struct Competition {
        address issuer;
        uint256 reward;
        address prover;
        CompetitionStatus status;
    }
    enum CompetitionStatus {
        NotOpened,
        Open,
        Verifying,
        Completed
    }
    mapping(uint256 => Competition) public competitions;
    uint256 private _competitionIdCounter;

    mapping(address => uint256) public activeCompetition;
    mapping(uint256 => EnumerableSet.AddressSet) private _competitors;

    event CompetitionOpened(uint256 indexed competitionId, address indexed issuer, uint256 reward);
    event CompetitionJoined(uint256 indexed competitionId, address indexed competitor);
    event CompetitionLeft(uint256 indexed competitionId, address indexed competitor);

    function getCompetitorCount(uint256 competitionId) external view returns (uint256) {
        return _competitors[competitionId].length();
    }

    function getCompetitors(uint256 competitionId) external view returns (address[] memory) {
        return _competitors[competitionId].values();
    }

    function openCompetition(uint256 reward) external payable returns (uint256) {
        require(msg.value == reward + verificationFee, "Incorrect ETH sent");
        require(reward > 0, "Reward must be greater than zero");

        _competitionIdCounter++;
        competitions[_competitionIdCounter] = Competition({
            issuer: msg.sender,
            reward: reward,
            prover: address(0),
            status: CompetitionStatus.Open
        });
        emit CompetitionOpened(_competitionIdCounter, msg.sender, reward);
        return _competitionIdCounter;
    }

    function joinCompetition(uint256 competitionId) external payable {
        Competition storage competition = competitions[competitionId];
        require(competition.status == CompetitionStatus.Open, "Competition not open");
        require(msg.value == minStake, "Incorrect stake amount");
        require(activeCompetition[msg.sender] == 0, "Already active in a competition");

        _competitors[competitionId].add(msg.sender);
        activeCompetition[msg.sender] = competitionId;
        emit CompetitionJoined(competitionId, msg.sender);
    }

    function leaveCompetition() external {
        require(activeCompetition[msg.sender] != 0, "Not active in any competition");
        Competition storage competition = competitions[activeCompetition[msg.sender]];
        require(competition.status == CompetitionStatus.Open, "Competition not open");

        _competitors[activeCompetition[msg.sender]].remove(msg.sender);
        activeCompetition[msg.sender] = 0;
        (bool sent, ) = msg.sender.call{value: minStake}("");
        require(sent, "ETH transfer failed");
        emit CompetitionLeft(activeCompetition[msg.sender], msg.sender);
    }
}