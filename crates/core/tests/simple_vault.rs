use alloy::{
    network::TransactionBuilder7702,
    node_bindings::Anvil,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
};
use ethereum_desktop_wallet_core::profile::{
    AssetId, VaultId,
    executor::{Executor, simple::SimpleExecutor},
    vault::{Vault, simple::SimpleVault},
};
use tracing::info;

sol!(
    #[sol(rpc)]
    SimpleDelegateContract,
    "../../contracts/out/SimpleDelegate.sol/SimpleDelegate.json",
);

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn test_simple_vault() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let anvil = Anvil::new().spawn();
    let rpc_url = anvil.endpoint();
    let signer = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let vault_signer = PrivateKeySigner::random();
    let executor_signer = PrivateKeySigner::from_slice(&anvil.nth_key(1).unwrap().to_bytes())?;

    let provider = ProviderBuilder::new()
        .wallet(signer.clone())
        .connect_http(rpc_url.parse()?)
        .erased();

    //? Deploy the SimpleDelegate contract
    let delegate_contract = SimpleDelegateContract::deploy(provider.clone()).await?;
    let delegate_address = *delegate_contract.address();
    info!(
        "Deployed SimpleDelegate contract at: {:?}",
        delegate_address
    );

    //? Create SimpleExecutor
    let executor = SimpleExecutor::new_with_delegate(
        executor_signer.clone(),
        provider.clone(),
        delegate_address,
    )
    .await?;
    info!("Created SimpleExecutor with ID {:?}", executor.id());

    //? Create and authorize SimpleVault
    let auth = SimpleVault::authorize_delegate(&vault_signer, &provider, delegate_address).await?;

    let tx = TransactionRequest::default()
        .to(signer.address())
        .with_authorization_list(vec![auth]);
    provider.send_transaction(tx).await?.get_receipt().await?;
    info!("Authorized SimpleVault");

    let vault =
        SimpleVault::new_with_delegate(vault_signer, provider.clone(), delegate_address).await?;
    info!("Created SimpleVault with ID {:?}", vault.id());

    //? Deposit into the vault
    info!("Depositing into the vault...");
    let deposit_asset = AssetId::Native;
    let deposit_amount = U256::from(10000);
    let deposit_calls = vault
        .deposit(signer.address(), &deposit_asset, deposit_amount)
        .await?;

    executor.execute(&deposit_calls).await?;
    info!("Deposit completed successfully.");

    //? Verify balance
    info!("Verifying balance after deposit...");
    let balance = vault.balance(&deposit_asset).await?;
    assert_eq!(
        balance, deposit_amount,
        "Expected vault balance to match deposited amount"
    );

    //? Withdraw
    info!("Withdrawing from the vault...");
    let withdraw_amount = U256::from(1234);
    let withdraw_target = Address::from_slice(&[1; 20]);
    let target_balance_before = provider.get_balance(withdraw_target).await?;

    let withdraw_calls = vault
        .withdraw(
            &VaultId::Address(withdraw_target),
            &deposit_asset,
            withdraw_amount,
        )
        .await?;

    executor.execute(&withdraw_calls).await?;
    info!("Withdrawal completed successfully.");

    //? Verify balance after withdrawal
    info!("Verifying balance after withdrawal...");
    let balance_after_withdrawal = vault.balance(&deposit_asset).await?;
    assert_eq!(
        balance_after_withdrawal,
        deposit_amount - withdraw_amount,
        "Expected vault balance to match after withdrawal"
    );

    //? Verify that the withdraw target received the funds
    let target_balance = provider.get_balance(withdraw_target).await?;
    assert_eq!(
        target_balance,
        target_balance_before + withdraw_amount,
        "Expected withdraw target to receive the funds"
    );

    Ok(())
}
