use std::sync::Arc;

use alloy_node_bindings::Anvil;
use alloy_primitives::{Address, U256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    database::memory::MemoryDatabase,
    network::{NetworkEndpoint, SimpleNetworkEndpoint},
    vault::{VaultId, tornadocash::TornadoVault},
};
use kohaku_fork_kit::pool::deploy_pool;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::utils::send_calls;

#[tokio::test]
#[ignore = "proof generation is slow; run with `cargo test --release -- --ignored`"]
async fn test_tornado_vault() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("info"))
        .try_init();

    let anvil = Anvil::new().spawn();
    let signer = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(anvil.endpoint().parse()?)
        .erased();
    let endpoint: Arc<dyn NetworkEndpoint> = Arc::new(SimpleNetworkEndpoint::new(provider.clone()));

    let pool = deploy_pool(provider.clone(), None).await?;
    info!("Deployed tornado pool at: {:?}", pool.address);
    let vault = TornadoVault::new(
        PrivateKeySigner::random(),
        Arc::new(MemoryDatabase::default()),
        endpoint,
    );

    info!("Depositing into the vault...");
    let deposit_calls = vault.deposit(&pool).await?;
    send_calls(&provider, &deposit_calls).await?;

    let notes = vault.notes().await?;
    assert_eq!(notes.len(), 1, "Expected one note after a single deposit");
    let note = &notes[0];
    assert_eq!(note.pool, pool);
    assert!(
        note.leaf_index.is_some(),
        "Expected the deposit to be mined"
    );
    assert!(!note.spent, "Expected the note to be unspent");

    info!("Withdrawing from the vault...");
    let withdraw_target = Address::from_slice(&[1; 20]);
    let target_balance_before = provider.get_balance(withdraw_target).await?;
    let withdraw_call = vault
        .withdraw(note, &VaultId::Address(withdraw_target))
        .await?;
    send_calls(&provider, &[withdraw_call]).await?;

    let target_balance = provider.get_balance(withdraw_target).await?;
    assert_eq!(
        target_balance,
        target_balance_before + U256::from(pool.amount_wei),
        "Expected withdraw target to receive the note's denomination"
    );
    let notes = vault.notes().await?;
    assert!(notes[0].spent, "Expected the note to be spent");

    Ok(())
}
