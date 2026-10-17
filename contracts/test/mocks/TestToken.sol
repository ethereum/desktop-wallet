// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {ERC20} from "@openzeppelin-contracts-5.7.0/token/ERC20/ERC20.sol";

/// An ERC-20 that mints its whole supply to the deployer.
contract TestToken is ERC20 {
    constructor(uint256 supply) ERC20("Test Token", "TEST") {
        _mint(msg.sender, supply);
    }
}
