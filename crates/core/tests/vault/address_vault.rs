use std::sync::Arc;

use alloy_node_bindings::Anvil;
use alloy_primitives::U256;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use edw_core::{
    asset::AssetId,
    database::{Database, memory::MemoryDatabase},
    factory::{BuildContext, Factory},
    network::{NetworkEndpoint, SimpleNetworkEndpoint},
    vault::{
        Balance, Vault,
        address::{
            ADDRESS_VAULT_TAG,
            db::{AddressVaultDb, DerivedAddress},
        },
    },
};

sol!(
    #[sol(rpc)]
    TestToken,
    "../../contracts/out/TestToken.sol/TestToken.json",
);

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn an_address_vault_reads_its_erc20_balance() -> Result<(), Box<dyn std::error::Error>> {
    let anvil = Anvil::new().spawn();
    let holder = PrivateKeySigner::from_slice(&anvil.first_key().to_bytes())?;
    let provider = ProviderBuilder::new()
        .wallet(holder.clone())
        .connect_http(anvil.endpoint().parse()?)
        .erased();
    let supply = U256::from(1_234_567);
    let token = TestToken::deploy(provider.clone(), supply).await?;

    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::default());
    db.put_derived_address(&DerivedAddress {
        address: holder.address(),
        index: 0,
    })
    .await?;
    let endpoint: Arc<dyn NetworkEndpoint> = Arc::new(SimpleNetworkEndpoint::new(provider));
    let vault =
        Factory::<dyn Vault>::build(ADDRESS_VAULT_TAG, BuildContext::new(endpoint, db)).await?;

    let balance = vault.balance(&AssetId::Erc20(*token.address())).await?;

    assert_eq!(balance, Balance::spendable(supply));
    Ok(())
}
