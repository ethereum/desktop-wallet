// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script} from "forge-std/Script.sol";
import {SimpleDelegate} from "../../src/simple_delegate/SimpleDelegate.sol";
import "forge-std/console.sol";

contract DeployScript is Script {
    function run() public {
        vm.startBroadcast();

        SimpleDelegate delegate = new SimpleDelegate{salt: bytes32(0)}();
        console.log("SimpleDelegate deployed at:", address(delegate));

        vm.stopBroadcast();
    }
}
