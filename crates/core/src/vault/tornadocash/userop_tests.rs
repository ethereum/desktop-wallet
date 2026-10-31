//! Local end-to-end coverage; requires `alto` and `anvil` on PATH.
use std::sync::Arc;

use alloy_node_bindings::Anvil;
use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use kohaku_fork_kit::{
    alto::AltoBuilder,
    entry_point::deploy_entry_point,
    paymaster::{allow_instance, deploy_paymaster},
    pool::deploy_pool,
    simple_account::deploy_simple_account,
};
use kohaku_tornadocash::Pool;

use super::{Asset, PaymasterWithdrawalRequest, TornadoVault, quote_paymaster_withdrawal};
use crate::{
    bundler::BundlerList,
    call::Call,
    database::memory::MemoryDatabase,
    network::{NetworkEndpoint, SimpleNetworkEndpoint},
};

async fn sponsored_pool(
    provider: &DynProvider,
    entrypoint: Address,
) -> Result<Pool, Box<dyn std::error::Error>> {
    let mut pool = deploy_pool(provider.clone(), None).await?;
    let paymaster = deploy_paymaster(
        provider.clone(),
        entrypoint,
        Address::repeat_byte(0x11),
        Address::repeat_byte(0x22),
    )
    .await?;
    allow_instance(provider.clone(), paymaster, pool.address).await?;
    pool.paymaster = Some(paymaster);
    Ok(pool)
}

#[tokio::test]
#[ignore = "requires alto on PATH and slow proof generation; run in release mode"]
async fn userop_tail_spends_unshielded_funds_and_rejects_unfunded_final_execution()
-> Result<(), Box<dyn std::error::Error>> {
    let anvil = Anvil::new().try_spawn()?;
    let provider = ProviderBuilder::new()
        .wallet(anvil.wallet().ok_or("no Anvil wallet")?)
        .connect_http(anvil.endpoint_url())
        .erased();
    let chain_id = provider.get_chain_id().await?;
    let entrypoint = deploy_entry_point(&provider).await?;
    deploy_simple_account(&provider).await?;
    let pool = sponsored_pool(&provider, entrypoint).await?;
    let endpoint: Arc<dyn NetworkEndpoint> = Arc::new(SimpleNetworkEndpoint::new(provider.clone()));
    let vault = TornadoVault::new(
        PrivateKeySigner::random(),
        Arc::new(MemoryDatabase::default()),
        endpoint,
    );
    for call in vault.deposit(&pool).await? {
        provider
            .send_transaction(
                alloy_rpc_types_eth::TransactionRequest::default()
                    .to(call.target)
                    .value(call.value)
                    .input(call.data.into()),
            )
            .await?
            .watch()
            .await?;
    }
    let notes = vault.notes().await?;
    let _alto = AltoBuilder::new(
        anvil.endpoint(),
        entrypoint,
        "0x4a3a02862ddcb260ed52d40ef03f8e3d78fa3d174b0ef333afdf1ffb4a648cd5",
        "0xdd4b2564c83ff7de602c39ffda1146055dc1814b07c083d7971722384f1f01a6",
    )
    .prefund(&provider)
    .await?
    .spawn()
    .await?;
    let urls = (3000..3020)
        .map(|port| format!("http://127.0.0.1:{port}").parse())
        .collect::<Result<Vec<_>, _>>()?;
    let owner = PrivateKeySigner::random();
    let recipient = Address::repeat_byte(0x42);
    let denomination = U256::from(pool.amount_wei);
    // This target succeeds with simulation funding but reverts with only real proceeds.
    // It accepts the call only when caller.balance > denomination - 1.
    let trap = Address::repeat_byte(0x55);
    let threshold = denomination - U256::from(1);
    let code: Bytes = format!("33317f{threshold:064x}10602a575f5ffd5b00").parse()?;
    provider
        .raw_request::<_, ()>("anvil_setCode".into(), (trap, code))
        .await?;
    let trapped = [Call::new(trap, Bytes::new(), U256::ZERO)];
    let rejected = quote_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: &vault,
        provider: provider.clone(),
        owner: owner.clone(),
        asset: &Asset::ETH,
        tail_calls: &trapped,
        notes: &notes,
        forward_to: None,
        minimum_forward: None,
        chain_id,
        bundlers: BundlerList::from_urls(chain_id, &urls)?,
    })
    .await?
    .submit()
    .await;
    let Err(error) = rejected else {
        return Err("unfunded final execution should fail".into());
    };
    assert!(
        error.to_string().contains("bundler") || error.to_string().contains("gas"),
        "{error}"
    );
    assert!(!vault.notes().await?[0].spent);
    // A successful tail consumes most of the withdrawn ETH.
    let value = denomination * U256::from(95) / U256::from(100);
    let calls = [Call::new(recipient, Bytes::new(), value)];
    let submitted = quote_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: &vault,
        provider: provider.clone(),
        owner: owner.clone(),
        asset: &Asset::ETH,
        tail_calls: &calls,
        notes: &notes,
        forward_to: None,
        minimum_forward: None,
        chain_id,
        bundlers: BundlerList::from_urls(chain_id, &urls)?,
    })
    .await?
    .submit()
    .await?;
    submitted.wait().await?;
    assert_eq!(provider.get_balance(recipient).await?, value);
    let remainder = provider.get_balance(owner.address()).await?;
    // The paymaster may refund the unused portion of the maximum fee during postOp.
    assert!(remainder >= denomination - submitted.fee - value);
    assert!(remainder <= denomination - value);
    assert!(vault.notes().await?[0].spent);
    check_forward(&provider, &vault, &pool, &urls).await?;
    Ok(())
}

async fn check_forward(
    provider: &DynProvider,
    vault: &TornadoVault<PrivateKeySigner>,
    pool: &Pool,
    urls: &[crate::bundler::Url],
) -> Result<(), Box<dyn std::error::Error>> {
    for call in vault.deposit(pool).await? {
        provider
            .send_transaction(
                alloy_rpc_types_eth::TransactionRequest::default()
                    .to(call.target)
                    .value(call.value)
                    .input(call.data.into()),
            )
            .await?
            .watch()
            .await?;
    }
    let notes = vault
        .notes()
        .await?
        .into_iter()
        .filter(|note| !note.spent)
        .collect::<Vec<_>>();
    let owner = PrivateKeySigner::random();
    let recipient = Address::repeat_byte(0x66);
    let submitted = quote_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault,
        provider: provider.clone(),
        owner: owner.clone(),
        asset: &Asset::ETH,
        tail_calls: &[],
        notes: &notes,
        forward_to: Some(recipient),
        minimum_forward: Some(U256::from(pool.amount_wei / 2)),
        chain_id: pool.chain_id,
        bundlers: BundlerList::from_urls(pool.chain_id, urls)?,
    })
    .await?
    .submit()
    .await?;
    submitted.wait().await?;
    assert_eq!(
        Some(provider.get_balance(recipient).await?),
        submitted.forwarded
    );
    check_prepared_reshield(provider, vault, pool, urls).await?;
    Ok(())
}

async fn check_prepared_reshield(
    provider: &DynProvider,
    vault: &TornadoVault<PrivateKeySigner>,
    source: &Pool,
    urls: &[crate::bundler::Url],
) -> Result<(), Box<dyn std::error::Error>> {
    let target = deploy_pool(provider.clone(), Some(10_u128.pow(16))).await?;
    // Prepare the deposit while the eventual fresh sender has no funds yet.
    let prepared = vault
        .prepare_deposits(std::slice::from_ref(&target))
        .await?;
    let nonce = prepared[0].nonce;
    let calls = prepared[0].calls.clone();
    for call in vault.deposit(source).await? {
        provider
            .send_transaction(
                alloy_rpc_types_eth::TransactionRequest::default()
                    .to(call.target)
                    .value(call.value)
                    .input(call.data.into()),
            )
            .await?
            .watch()
            .await?;
    }
    let notes: Vec<_> = vault
        .notes()
        .await?
        .into_iter()
        .filter(|note| note.pool == *source && !note.spent)
        .collect();
    let owner = PrivateKeySigner::random();
    assert_eq!(provider.get_balance(owner.address()).await?, U256::ZERO);
    let submitted = quote_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault,
        provider: provider.clone(),
        owner: owner.clone(),
        asset: &Asset::ETH,
        tail_calls: &calls,
        notes: &notes,
        forward_to: None,
        minimum_forward: None,
        chain_id: source.chain_id,
        bundlers: BundlerList::from_urls(source.chain_id, urls)?,
    })
    .await?
    .submit()
    .await?;
    submitted.wait().await?;
    let held = vault.notes().await?;
    let reshielded = held
        .iter()
        .find(|note| note.pool == target && note.nonce == nonce)
        .ok_or("prepared note was lost")?;
    assert!(
        reshielded.leaf_index.is_some(),
        "prepared note must be recognized once its call is mined"
    );
    assert!(!reshielded.spent);
    assert_eq!(
        provider.get_balance(target.address).await?,
        U256::from(target.amount_wei)
    );
    Ok(())
}
