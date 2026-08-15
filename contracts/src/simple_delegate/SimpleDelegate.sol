// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import "@openzeppelin-contracts-5.7.0/utils/cryptography/EIP712.sol";

struct Call {
    address target;
    uint256 value;
    bytes data;
}

contract SimpleDelegate is EIP712 {
    /// @custom:storage-location erc7201:simple_delegate.main
    struct Storage {
        uint256 nonce;
    }

    // keccak256(abi.encode(uint256(keccak256("simple_delegate.main")) - 1)) & ~bytes32(uint256(0xff));
    bytes32 private constant _STORAGE =
        0xbfbfcd6df78e796c7f6a4ffb712d0537d34f4debe7adeafdef6a66872f365f00;

    bytes32 private constant CALL_TYPEHASH =
        keccak256("Call(address target,uint256 value,bytes data)");

    bytes32 private constant EXECUTE_BATCH_TYPEHASH =
        keccak256(
            "ExecuteBatch(Call[] calls,uint256 nonce)Call(address target,uint256 value,bytes data)"
        );

    error InvalidSignature();
    error CallReverted(address target, uint256 value, bytes data, bytes ret);

    constructor() EIP712("SimpleDelegate", "1") {}

    receive() external payable {}

    function nonce() external view returns (uint256) {
        return _storage().nonce;
    }

    function executeBatch(
        Call[] calldata calls,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        Storage storage $ = _storage();
        uint256 _nonce = $.nonce;

        bytes32 digest = _hashTypedDataV4(hashBatch(calls, _nonce));
        address recovered = ecrecover(digest, v, r, s);

        if (recovered != address(this)) {
            revert InvalidSignature();
        }

        $.nonce = _nonce + 1;
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory ret) = calls[i].target.call{
                value: calls[i].value
            }(calls[i].data);

            if (!ok) {
                revert CallReverted(
                    calls[i].target,
                    calls[i].value,
                    calls[i].data,
                    ret
                );
            }
        }
    }

    function hashBatch(
        Call[] calldata calls,
        uint256 _nonce
    ) public pure returns (bytes32) {
        bytes32[] memory callHashes = new bytes32[](calls.length);
        for (uint256 i = 0; i < calls.length; i++) {
            callHashes[i] = hashCall(calls[i]);
        }
        return
            keccak256(
                abi.encode(
                    EXECUTE_BATCH_TYPEHASH,
                    keccak256(abi.encodePacked(callHashes)),
                    _nonce
                )
            );
    }

    function hashCall(Call calldata call) public pure returns (bytes32) {
        return
            keccak256(
                abi.encode(
                    CALL_TYPEHASH,
                    call.target,
                    call.value,
                    keccak256(call.data)
                )
            );
    }

    function _storage() private pure returns (Storage storage $) {
        assembly ("memory-safe") {
            $.slot := _STORAGE
        }
    }
}
