// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract MultisigTreasury {
    event Deposit(address indexed sender, uint amount, uint balance);
    event SubmitTransaction(address indexed owner, uint indexed txIndex, address indexed to, uint value, bytes data);
    event ConfirmTransaction(address indexed owner, uint indexed txIndex);
    event RevokeConfirmation(address indexed owner, uint indexed txIndex);
    event ExecuteTransaction(address indexed owner, uint indexed txIndex);
    event LimitsUpdated(uint dailyLimit, uint weeklyLimit, uint threshold);
    event EmergencyFrozen(address indexed by);
    event EmergencyUnfrozen(address indexed by);

    address[] public owners;
    mapping(address => bool) public isOwner;
    uint public required;

    uint public dailyLimit;
    uint public weeklyLimit;
    uint public threshold;

    bool public frozen;

    struct Transaction {
        address to;
        uint value;
        bytes data;
        bool executed;
        uint numConfirmations;
        uint created;
    }

    Transaction[] public transactions;
    mapping(uint => mapping(address => bool)) public isConfirmed;

    // tracking spend windows
    uint public dayWindowStart;
    uint public daySpent;
    uint public weekWindowStart;
    uint public weekSpent;

    modifier onlyOwner() { require(isOwner[msg.sender], "not owner"); _; }
    modifier notFrozen() { require(!frozen, "frozen"); _; }

    constructor(address[] memory _owners, uint _required, uint _dailyLimit, uint _weeklyLimit, uint _threshold) {
        require(_owners.length > 0, "owners required");
        require(_required > 0 && _required <= _owners.length, "invalid required");
        for (uint i = 0; i < _owners.length; i++) {
            address o = _owners[i];
            require(o != address(0), "invalid owner");
            require(!isOwner[o], "owner not unique");
            isOwner[o] = true;
            owners.push(o);
        }
        required = _required;
        dailyLimit = _dailyLimit;
        weeklyLimit = _weeklyLimit;
        threshold = _threshold;
        dayWindowStart = block.timestamp / 1 days;
        weekWindowStart = block.timestamp / 1 weeks;
    }

    receive() external payable {
        emit Deposit(msg.sender, msg.value, address(this).balance);
    }

    function submitTransaction(address _to, uint _value, bytes calldata _data) external onlyOwner returns (uint) {
        transactions.push(Transaction({ to: _to, value: _value, data: _data, executed: false, numConfirmations: 0, created: block.timestamp }));
        uint txIndex = transactions.length - 1;
        emit SubmitTransaction(msg.sender, txIndex, _to, _value, _data);
        return txIndex;
    }

    function confirmTransaction(uint _txIndex) external onlyOwner {
        require(_txIndex < transactions.length, "tx does not exist");
        Transaction storage txn = transactions[_txIndex];
        require(!txn.executed, "already executed");
        require(!isConfirmed[_txIndex][msg.sender], "already confirmed");

        // Allow confirming unfreeze calls even when frozen; all other txs require unfrozen state.
        if (frozen) {
            bytes4 selector;
            if (txn.data.length >= 4) {
                bytes memory d = txn.data;
                assembly { selector := mload(add(d, 32)) }
            }
            bool isUnfreezeCall = (txn.to == address(this) && txn.data.length >= 4 && selector == this.unfreezeInternal.selector);
            require(isUnfreezeCall, "frozen");
        }

        isConfirmed[_txIndex][msg.sender] = true;
        txn.numConfirmations += 1;
        emit ConfirmTransaction(msg.sender, _txIndex);
    }

    function revokeConfirmation(uint _txIndex) external onlyOwner notFrozen {
        require(_txIndex < transactions.length, "tx does not exist");
        Transaction storage txn = transactions[_txIndex];
        require(!txn.executed, "already executed");
        require(isConfirmed[_txIndex][msg.sender], "not confirmed");
        isConfirmed[_txIndex][msg.sender] = false;
        txn.numConfirmations -= 1;
        emit RevokeConfirmation(msg.sender, _txIndex);
    }

    function executeTransaction(uint _txIndex) external {
        require(_txIndex < transactions.length, "tx does not exist");
        Transaction storage txn = transactions[_txIndex];
        require(!txn.executed, "already executed");

        // Special-case: if this is an internal unfreeze call, require full multisig
        bytes4 selector;
        if (txn.data.length >= 4) {
            bytes memory d = txn.data;
            assembly { selector := mload(add(d, 32)) }
        }
        bool isUnfreezeCall = (txn.to == address(this) && txn.data.length >= 4 && selector == this.unfreezeInternal.selector);

        // Unfreeze calls may execute while frozen; all other calls require unfrozen state.
        if (!isUnfreezeCall) {
            require(!frozen, "frozen");
        }

        if (isUnfreezeCall) {
            require(txn.numConfirmations >= required, "insufficient confirmations for unfreeze");
        } else if (txn.value > threshold) {
            require(txn.numConfirmations >= required, "insufficient confirmations for large tx");
        } else {
            require(txn.numConfirmations >= 1, "requires at least one confirmation");
        }

        // Update windows
        uint currentDay = block.timestamp / 1 days;
        if (dayWindowStart != currentDay) {
            dayWindowStart = currentDay;
            daySpent = 0;
        }
        uint currentWeek = block.timestamp / 1 weeks;
        if (weekWindowStart != currentWeek) {
            weekWindowStart = currentWeek;
            weekSpent = 0;
        }

        // Enforce limits if set (non-zero)
        if (dailyLimit > 0) {
            require(daySpent + txn.value <= dailyLimit, "exceeds daily limit");
        }
        if (weeklyLimit > 0) {
            require(weekSpent + txn.value <= weeklyLimit, "exceeds weekly limit");
        }

        txn.executed = true;
        daySpent += txn.value;
        weekSpent += txn.value;

        (bool success, ) = txn.to.call{ value: txn.value }(txn.data);
        require(success, "tx failed");
        emit ExecuteTransaction(msg.sender, _txIndex);
    }

    function updateLimits(uint _dailyLimit, uint _weeklyLimit, uint _threshold) external onlyOwner {
        dailyLimit = _dailyLimit;
        weeklyLimit = _weeklyLimit;
        threshold = _threshold;
        emit LimitsUpdated(_dailyLimit, _weeklyLimit, _threshold);
    }

    // Emergency freeze: immediate and fast
    function emergencyFreeze() external onlyOwner {
        frozen = true;
        emit EmergencyFrozen(msg.sender);
    }

    // Unfreeze must be performed via an on-chain multisig transaction targeting this contract:
    // submitTransaction(address(this), 0, abi.encodeWithSelector(this.unfreezeInternal.selector))
    function unfreezeInternal() external {
        require(msg.sender == address(this), "only self");
        frozen = false;
        emit EmergencyUnfrozen(address(this));
    }

    // Helpers
    function getOwners() external view returns (address[] memory) { return owners; }
    function getTransactionCount() external view returns (uint) { return transactions.length; }
    function getTransaction(uint _txIndex) external view returns (address to, uint value, bytes memory data, bool executed, uint numConfirmations, uint created) {
        Transaction storage t = transactions[_txIndex];
        return (t.to, t.value, t.data, t.executed, t.numConfirmations, t.created);
    }

    /**
     * @dev Storage gap to reserve space for future state variables when upgrading
     * via a proxy pattern. Reduces storage collision risk across upgrade versions.
     * Size: 50 slots (standard OpenZeppelin pattern).
     */
    uint256[50] private __gap;
}
