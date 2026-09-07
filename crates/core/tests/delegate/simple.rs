use std::sync::Arc;

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::ProviderBuilder;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolStruct, sol};

mod common;

sol!(
    #[sol(rpc)]
    SimpleDelegateContract,
    "../../contracts/out/SimpleDelegate.sol/SimpleDelegate.json",
);

mod eip712_types {
    use alloy_sol_types::sol;

    sol!(
        struct Call {
            address target;
            uint256 value;
            bytes data;
        }

        struct ExecuteBatch {
            Call[] calls;
            uint256 nonce;
        }
    );
}

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn test_typehashes_match_contract() -> Result<(), Box<dyn std::error::Error>> {
    let anvil = common::devnet();
    let signer = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let provider = Arc::new(
        ProviderBuilder::new()
            .wallet(signer)
            .connect_http(anvil.endpoint().parse()?),
    );

    let delegate_contract = SimpleDelegateContract::deploy(provider.clone()).await?;

    let call = SimpleDelegateContract::Call {
        target: Address::from_slice(&[2; 20]),
        value: U256::from(1234),
        data: Bytes::new(),
    };
    let nonce = U256::from(0);

    //? Struct hashes as computed by the deployed Solidity contract.
    let onchain_call_hash = delegate_contract.hashCall(call.clone()).call().await?;
    let onchain_batch_hash = delegate_contract
        .hashBatch(vec![call.clone()], nonce)
        .call()
        .await?;

    //? The same struct hashes as computed independently by Rust's EIP-712 derivation.
    let local_call = eip712_types::Call {
        target: call.target,
        value: call.value,
        data: call.data.clone(),
    };
    let local_call_hash = local_call.eip712_hash_struct();
    let local_batch = eip712_types::ExecuteBatch {
        calls: vec![local_call],
        nonce,
    };
    let local_batch_hash = local_batch.eip712_hash_struct();

    assert_eq!(
        onchain_call_hash, local_call_hash,
        "Call struct hash mismatch between Rust and the SimpleDelegate contract"
    );
    assert_eq!(
        onchain_batch_hash, local_batch_hash,
        "ExecuteBatch struct hash mismatch between Rust and the SimpleDelegate contract"
    );

    Ok(())
}
