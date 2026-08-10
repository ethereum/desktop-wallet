use std::sync::Arc;

use alloy_network::TransactionBuilder7702;
use alloy_node_bindings::Anvil;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use ethereum_desktop_wallet_core::{database::Database, factory::BuildContext, profile::Profile};
use ethereum_desktop_wallet_wallet::{
    database::memory::MemoryDatabase, simple_executor::SimpleExecutor,
    simple_profile::SimpleProfile, simple_vault::SimpleVault,
};
use tracing::info;

sol!(
    #[sol(rpc)]
    SimpleDelegateContract,
    "../../contracts/out/SimpleDelegate.sol/SimpleDelegate.json",
);

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn test_simple_profile() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let anvil = Anvil::new().spawn();
    let rpc_url = anvil.endpoint();
    let sponsor = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let executor_signer = PrivateKeySigner::from_slice(&anvil.nth_key(1).unwrap().to_bytes())?;
    let vault_signer = PrivateKeySigner::random();

    let provider = Arc::new(
        ProviderBuilder::new()
            .wallet(sponsor.clone())
            .connect_http(rpc_url.parse()?),
    );

    //? Deploy the SimpleDelegate contract
    let delegate_contract = SimpleDelegateContract::deploy(provider.clone()).await?;
    let implementation = *delegate_contract.address();
    info!("Deployed SimpleDelegate contract at: {:?}", implementation);

    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::default());

    //? Construct a profile with a default executor. `SimpleExecutor` authorizes
    //? itself, so no sponsor transaction is needed here.
    let mut profile = SimpleProfile::new(
        provider.clone(),
        db.clone(),
        |ctx: BuildContext| async move {
            SimpleExecutor::new_with_implementation(
                executor_signer,
                implementation,
                ctx.provider,
                ctx.db,
            )
            .await
        },
    )
    .await?;
    info!(
        "Created profile with default executor {:?}",
        profile.default_executor.1.id()
    );

    //? Unlike `SimpleExecutor`, `SimpleVault` can't pay for its own authorization,
    //? so the sponsor submits it on the vault signer's behalf before construction.
    let auth =
        SimpleVault::authorize_implementation(&vault_signer, implementation, provider.as_ref())
            .await?;
    let tx = TransactionRequest::default()
        .to(sponsor.address())
        .with_authorization_list(vec![auth]);
    provider.send_transaction(tx).await?.get_receipt().await?;

    profile
        .add_vault(|ctx: BuildContext| async move {
            SimpleVault::new_with_implementation(vault_signer, implementation, ctx.provider, ctx.db)
                .await
        })
        .await?;
    info!("Added vault {:?} to profile", profile.vaults[0].1.id());

    //? Reload the profile from the database and verify every object was
    //? reconstructed without error.
    let loaded = SimpleProfile::load(provider.clone(), db.clone()).await?;

    assert_eq!(
        loaded.default_executor.1.id(),
        profile.default_executor.1.id(),
        "Expected the reloaded default executor to match the original"
    );
    assert_eq!(
        loaded.vaults.len(),
        profile.vaults.len(),
        "Expected the reloaded vault count to match the original"
    );
    assert_eq!(
        loaded.vaults[0].1.id(),
        profile.vaults[0].1.id(),
        "Expected the reloaded vault to match the original"
    );

    Ok(())
}
