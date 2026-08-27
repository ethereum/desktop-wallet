use std::{sync::Arc, time::Duration};

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use edw_core::{call::Call, executor::Executor};
use edw_wallet::{database::memory::MemoryDatabase, simple_executor::SimpleExecutor};
use tracing::info;

mod common;

/// How long a test waits for a submitted call to be mined.
const MINING_TIMEOUT: Duration = Duration::from_secs(30);

sol!(
    #[sol(rpc)]
    SimpleDelegateContract,
    "../../contracts/out/SimpleDelegate.sol/SimpleDelegate.json",
);

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn test_simple_executor() -> Result<(), Box<dyn std::error::Error>> {
    common::init_tracing();

    let anvil = common::devnet();
    let rpc_url = anvil.endpoint();
    let signer = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let executor_signer = PrivateKeySigner::from_slice(
        &anvil
            .nth_key(1)
            .ok_or("Failed to get executor signer")?
            .to_bytes(),
    )?;

    let provider = Arc::new(
        ProviderBuilder::new()
            .wallet(signer.clone())
            .connect_http(rpc_url.parse()?),
    );

    //? Deploy the SimpleDelegate contract
    let delegate_contract = SimpleDelegateContract::deploy(provider.clone()).await?;
    let delegate_address = *delegate_contract.address();
    info!(
        "Deployed SimpleDelegate contract at: {:?}",
        delegate_address
    );

    //? Create SimpleExecutor
    let executor = SimpleExecutor::new_with_implementation(
        executor_signer.clone(),
        delegate_address,
        provider.clone(),
        Arc::new(MemoryDatabase::default()),
    )
    .await?;
    info!("Created SimpleExecutor with ID {:?}", executor.id());

    //? Make an arbitrary call through the executor
    info!("Sending calls through SimpleExecutor...");
    let nonce_before_call = provider.get_transaction_count(executor.address()).await?;

    let target_1 = Address::from_slice(&[2; 20]);
    let target_2 = Address::from_slice(&[3; 20]);
    let value_1 = U256::from(1234);
    let value_2 = U256::from(5678);
    let call = executor
        .execute(&[
            Call::new(target_1, Bytes::new(), value_1),
            Call::new(target_2, Bytes::new(), value_2),
        ])
        .await?;
    executor.await_call(call, MINING_TIMEOUT).await?;
    info!("Sent SimpleExecutor calls");

    //? Verify the call was executed by checking the nonce of the SimpleDelegate contract
    info!("Verifying SimpleDelegate's address nonce after sending calls...");
    let nonce = provider.get_transaction_count(executor.address()).await?;
    assert_eq!(
        nonce,
        nonce_before_call + 1,
        "Expected nonce to increment by 1 after sending a call"
    );

    //? Verify the balance of the target address was updated
    info!("Verifying balances of target addresses...");
    let balance = provider.get_balance(target_1).await?;
    assert_eq!(
        value_1, balance,
        "Expected target_1 address to receive the value sent"
    );
    let balance = provider.get_balance(target_2).await?;
    assert_eq!(
        value_2, balance,
        "Expected target_2 address to receive the value sent"
    );

    Ok(())
}
