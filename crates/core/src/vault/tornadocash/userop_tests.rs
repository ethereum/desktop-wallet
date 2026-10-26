//! Local end-to-end coverage; requires `alto` and `anvil` on PATH.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use alloy_node_bindings::Anvil;
use alloy_primitives::{Address, B256, Bytes, Signature, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use kohaku_fork_kit::{
    alto::AltoBuilder,
    entry_point::deploy_entry_point,
    paymaster::{deploy_fee_adapter, deploy_paymaster},
    pool::deploy_pool,
    simple_account::deploy_simple_account,
};
use kohaku_tornadocash::{PaymasterInfo, Pool};

use super::{Asset, PaymasterWithdrawalRequest, TornadoVault, submit_paymaster_withdrawal};
use crate::{
    bundler::BundlerList,
    call::Call,
    database::memory::MemoryDatabase,
    network::{NetworkEndpoint, SimpleNetworkEndpoint},
};

struct CountingSigner {
    key: PrivateKeySigner,
    requests: AtomicUsize,
}
#[async_trait::async_trait]
impl Signer for CountingSigner {
    async fn sign_hash(&self, hash: &B256) -> alloy_signer::Result<Signature> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.key.sign_hash(hash).await
    }
    fn address(&self) -> Address {
        self.key.address()
    }
    fn chain_id(&self) -> Option<u64> {
        self.key.chain_id()
    }
    fn set_chain_id(&mut self, id: Option<u64>) {
        self.key.set_chain_id(id);
    }
}

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
    let adapter = deploy_fee_adapter(provider.clone(), paymaster, pool.address).await?;
    pool.paymaster = Some(PaymasterInfo {
        address: paymaster,
        adapter,
    });
    Ok(pool)
}

#[tokio::test]
#[ignore = "requires alto on PATH and slow proof generation; run in release mode"]
async fn userop_tail_spends_unshielded_funds_and_signs_only_after_real_simulation()
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
    let owner = CountingSigner {
        key: PrivateKeySigner::random(),
        requests: AtomicUsize::new(0),
    };
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
    let rejected = submit_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: &vault,
        provider: provider.clone(),
        owner: &owner,
        asset: &Asset::ETH,
        tail_calls: &trapped,
        notes: &notes,
        forward_to: None,
        minimum_forward: None,
        chain_id,
        bundlers: BundlerList::from_urls(chain_id, &urls)?,
    })
    .await;
    let Err(error) = rejected else {
        return Err("unfunded final execution should fail".into());
    };
    assert!(
        error.to_string().contains("final operation simulation"),
        "{error}"
    );
    assert_eq!(owner.requests.load(Ordering::SeqCst), 0);
    assert!(!vault.notes().await?[0].spent);
    // The seed quote leaves only 90%; this call deliberately needs 95%.
    let value = denomination * U256::from(95) / U256::from(100);
    let calls = [Call::new(recipient, Bytes::new(), value)];
    let submitted = submit_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: &vault,
        provider: provider.clone(),
        owner: &owner,
        asset: &Asset::ETH,
        tail_calls: &calls,
        notes: &notes,
        forward_to: None,
        minimum_forward: None,
        chain_id,
        bundlers: BundlerList::from_urls(chain_id, &urls)?,
    })
    .await?;
    // UserOp and 7702 authorization, with no signing during either fee quote.
    assert_eq!(owner.requests.load(Ordering::SeqCst), 2);
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
    let submitted = submit_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault,
        provider: provider.clone(),
        owner: &owner,
        asset: &Asset::ETH,
        tail_calls: &[],
        notes: &notes,
        forward_to: Some(recipient),
        minimum_forward: Some(U256::from(pool.amount_wei / 2)),
        chain_id: pool.chain_id,
        bundlers: BundlerList::from_urls(pool.chain_id, urls)?,
    })
    .await?;
    submitted.wait().await?;
    assert_eq!(
        Some(provider.get_balance(recipient).await?),
        submitted.forwarded
    );
    Ok(())
}
