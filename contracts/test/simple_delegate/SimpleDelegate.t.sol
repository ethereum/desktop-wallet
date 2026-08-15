// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {
    Call,
    SimpleDelegate
} from "../../src/simple_delegate/SimpleDelegate.sol";

contract SimpleDelegateTest is Test {
    uint256 signerKey = 0xA11CE;
    address signer;

    SimpleDelegate delegate;
    MockTarget target;

    constructor() {
        signer = vm.addr(signerKey);
        vm.etch(signer, address(new SimpleDelegate()).code);
        delegate = SimpleDelegate(payable(signer));

        target = new MockTarget();
        vm.deal(signer, 1 ether);
    }

    function test_ExecuteBatch_CallsTargetWithValueAndData() public {
        Call[] memory calls = new Call[](1);
        calls[0] = Call({
            target: address(target),
            value: 1 ether,
            data: abi.encodeCall(MockTarget.succeed, ())
        });

        (uint8 v, bytes32 r, bytes32 s) = _sign(calls, signerKey);
        delegate.executeBatch(calls, v, r, s);

        assertEq(target.lastValue(), 1 ether);
        assertEq(target.lastData(), abi.encodeCall(MockTarget.succeed, ()));
    }

    function test_ExecuteBatch_RevertsOnReplay() public {
        Call[] memory calls = new Call[](0);

        (uint8 v, bytes32 r, bytes32 s) = _sign(calls, signerKey);
        delegate.executeBatch(calls, v, r, s);

        vm.expectRevert(SimpleDelegate.InvalidSignature.selector);
        delegate.executeBatch(calls, v, r, s);
    }

    function test_ExecuteBatch_RevertsOnInvalidSigner() public {
        Call[] memory calls = new Call[](0);
        uint256 wrongKey = 0xB0B;

        (uint8 v, bytes32 r, bytes32 s) = _sign(calls, wrongKey);

        vm.expectRevert(SimpleDelegate.InvalidSignature.selector);
        delegate.executeBatch(calls, v, r, s);
    }

    function test_ExecuteBatch_RevertsOnSubcallRevert() public {
        Call[] memory calls = new Call[](1);
        calls[0] = Call({
            target: address(target),
            value: 0,
            data: abi.encodeCall(MockTarget.fail, ())
        });

        (uint8 v, bytes32 r, bytes32 s) = _sign(calls, signerKey);

        vm.expectRevert(
            abi.encodeWithSelector(
                SimpleDelegate.CallReverted.selector,
                calls[0].target,
                calls[0].value,
                calls[0].data,
                abi.encodeWithSelector(MockTarget.Fail.selector)
            )
        );
        delegate.executeBatch(calls, v, r, s);
    }

    function _sign(
        Call[] memory calls,
        uint256 pk
    ) private view returns (uint8 v, bytes32 r, bytes32 s) {
        bytes32 domainSeparator = keccak256(
            abi.encode(
                keccak256(
                    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
                ),
                keccak256(bytes("SimpleDelegate")),
                keccak256(bytes("1")),
                block.chainid,
                address(delegate)
            )
        );
        bytes32 structHash = delegate.hashBatch(calls, delegate.nonce());
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", domainSeparator, structHash)
        );
        (v, r, s) = vm.sign(pk, digest);
    }
}

contract MockTarget {
    uint256 public lastValue;
    bytes public lastData;

    error Fail();

    function succeed() external payable {
        lastValue = msg.value;
        lastData = msg.data;
    }

    function fail() external pure {
        revert Fail();
    }
}
