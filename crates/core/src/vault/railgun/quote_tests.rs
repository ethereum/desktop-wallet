//! Exercise the native paymaster path with public fixture notes and dummy proofs.
//! No chain, artifacts, private signature or PPOI service is contacted.
use std::sync::Mutex;

use alloy_primitives::{B256, Bytes, U64};
use alloy_sol_types::SolValue;
use alloy_transport::mock::Asserter;
use kohaku_railgun::{
    RailgunCheckpoint,
    abis::railgun::Transaction,
    indexer::IndexedAccountState,
    merkle_tree::UtxoMerkleTree,
    note::{Note, utxo::UtxoNote},
    poi::types::BlindedCommitmentType,
};
use kohaku_railgun_poi::{PoiCheckpoint, PoiListState, poi_tree::PoiMerkleTree};
use railgun_userop_kit::{
    bundler::{BundlerError, GasPrice},
    signable_user_operation::SignableUserOperation,
    user_operation::{UserOperationHash, UserOperationReceipt},
};

use super::*;
use crate::{database::memory::MemoryDatabase, test_support::mocked_provider};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
struct InspectBundler {
    ops: Mutex<Vec<SignableUserOperation>>,
}

sol! {
    struct Envelope { address adapter; bytes adapterData; }
    struct Adapter { bytes16 random; address asset; uint120 value; Transaction[] transactions; }
}

#[async_trait::async_trait]
impl Bundler for InspectBundler {
    async fn estimate_gas(
        &self,
        op: &SignableUserOperation,
        overrides: Option<&alloy_rpc_types_eth::state::StateOverride>,
    ) -> std::result::Result<UserOperationGasEstimate, BundlerError> {
        assert!(overrides.is_some());
        self.ops.lock().unwrap().push(op.clone());
        Ok(UserOperationGasEstimate {
            pre_verification_gas: 100,
            verification_gas_limit: 100,
            call_gas_limit: 100,
            paymaster_verification_gas_limit: Some(100),
            paymaster_post_op_gas_limit: Some(100),
            max_fee_per_gas: 2,
            max_priority_fee_per_gas: 1,
        })
    }
    async fn gas_price(&self) -> std::result::Result<Option<GasPrice>, BundlerError> {
        Ok(None)
    }
    async fn send_user_operation(
        &self,
        _: &SignedUserOperation,
    ) -> std::result::Result<UserOperationHash, BundlerError> {
        Err(BundlerError::Other("quote must not submit".into()))
    }
    async fn receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<Option<UserOperationReceipt>, BundlerError> {
        Err(BundlerError::Other("quote must not submit".into()))
    }
    async fn wait_for_receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<UserOperationReceipt, BundlerError> {
        Err(BundlerError::Other("quote must not submit".into()))
    }
}

async fn fixture(valid_weth: bool) -> (RailgunVault, InspectBundler, Address) {
    let asserter = Asserter::new();
    asserter.push_success(&U64::from(1));
    for _ in 0..2 {
        asserter.push_success(&Bytes::from(vec![0; 32]));
        asserter.push_success(&U64::ZERO);
    }
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    let vault = RailgunVault::open(
        &Mnemonic::parse(PHRASE).unwrap(),
        0,
        1,
        db,
        mocked_provider(&asserter),
        None,
    )
    .unwrap();
    let token = Address::repeat_byte(0x11);
    let notes = vec![
        UtxoNote::new(
            0,
            0,
            vault.signer.clone(),
            RailgunAsset::Erc20(token),
            1000,
            [1; 16],
            "",
            BlindedCommitmentType::Transact,
        ),
        UtxoNote::new(
            0,
            1,
            vault.signer.clone(),
            RailgunAsset::Erc20(vault.chain.wrapped_base_token),
            1_000_000_000_000_000_000,
            [2; 16],
            "",
            BlindedCommitmentType::Transact,
        ),
    ];
    let mut tree = UtxoMerkleTree::new(0);
    tree.insert_leaves(&notes.iter().map(Note::hash).collect::<Vec<_>>(), 0)
        .unwrap();
    let mut railgun = RailgunCheckpoint::default();
    railgun.utxo.trees.insert(0, tree.state());
    railgun.utxo.accounts.insert(
        vault.address(),
        IndexedAccountState {
            notes: notes.clone(),
            ..Default::default()
        },
    );
    let mut poi = PoiCheckpoint::default();
    let mut tree = PoiMerkleTree::new(0);
    let leaves = notes
        .iter()
        .take(if valid_weth { 2 } else { 1 })
        .map(|n| n.blinded_commitment)
        .collect::<Vec<_>>();
    tree.insert_leaves(&leaves, 0).unwrap();
    for list in &vault.chain.list_keys {
        poi.lists.0.insert(
            list.clone(),
            PoiListState {
                trees: std::collections::HashMap::from([(0, tree.state())]),
                next_index: leaves.len() as u64,
            },
        );
    }
    let state = StoredState {
        version: 1,
        chain_id: 1,
        address: vault.address(),
        wallet: PoiWalletCheckpoint { railgun, poi },
        pending: None,
    };
    vault
        .db
        .put(CHECKPOINT, &serde_json::to_vec(&state).unwrap())
        .await
        .unwrap();
    (
        vault,
        InspectBundler {
            ops: Mutex::new(Vec::new()),
        },
        token,
    )
}

#[tokio::test]
async fn erc20_unshield_fee_is_selected_from_private_weth_and_quotes_do_not_create_poi_work() {
    let (vault, bundler, token) = fixture(true).await;
    let mut session = vault.session().await.unwrap();
    session.observed = true;
    let owner = AccountSigner::from_bytes(&B256::repeat_byte(1)).unwrap();
    let quote = session
        .quote(
            Movement::Unshield {
                token,
                amount: U256::from(500),
                unwrap_native: false,
                calls: vec![],
            },
            &owner,
            &bundler,
        )
        .await
        .unwrap();
    assert_eq!(quote.fee, U256::from(1000));
    assert_eq!(quote.protocol_fee, U256::from(1));
    assert!(session.pending().is_none());
    assert!(session.poi_pending().is_empty());
    let ops = bundler.ops.lock().unwrap();
    assert_eq!(ops.len(), 2);
    for op in &*ops {
        let envelope = Envelope::abi_decode(op.user_op.paymaster_data.as_ref().unwrap()).unwrap();
        let data = Adapter::abi_decode(&envelope.adapterData).unwrap();
        assert_eq!(data.asset, vault.chain.wrapped_base_token);
        assert_eq!(data.transactions.len(), 2);
        assert!(
            data.transactions
                .iter()
                .any(|tx| tx.unshieldPreimage.token.tokenAddress == token)
        );
        assert!(op.user_op.call_data.is_empty());
    }
}

#[tokio::test]
async fn private_transfer_keeps_execution_empty_and_uses_a_weth_fee() {
    let (vault, bundler, token) = fixture(true).await;
    let mut session = vault.session().await.unwrap();
    session.observed = true;
    let owner = AccountSigner::from_bytes(&B256::repeat_byte(1)).unwrap();
    let quote = session
        .quote(
            Movement::Transfer {
                token,
                amount: U256::from(500),
                recipient: vault.address(),
                memo: "fixture".into(),
            },
            &owner,
            &bundler,
        )
        .await
        .unwrap();
    assert_eq!(quote.protocol_fee, U256::ZERO);
    assert!(quote.execution_calls().is_empty());
    for op in &*bundler.ops.lock().unwrap() {
        assert!(op.user_op.call_data.is_empty());
        let envelope = Envelope::abi_decode(op.user_op.paymaster_data.as_ref().unwrap()).unwrap();
        let data = Adapter::abi_decode(&envelope.adapterData).unwrap();
        assert_eq!(data.asset, vault.chain.wrapped_base_token);
        assert!(data.transactions.iter().all(|tx| matches!(
            tx.boundParams.unshield,
            kohaku_railgun::abis::railgun::UnshieldType::NONE
        )));
    }
}

#[tokio::test]
async fn unavailable_weth_never_uses_the_unshield_token_for_fees() {
    let (vault, bundler, token) = fixture(false).await;
    let mut session = vault.session().await.unwrap();
    session.observed = true;
    let owner = AccountSigner::from_bytes(&B256::repeat_byte(1)).unwrap();
    let result = session
        .quote(
            Movement::Unshield {
                token,
                amount: U256::from(500),
                unwrap_native: false,
                calls: vec![],
            },
            &owner,
            &bundler,
        )
        .await;
    assert!(
        matches!(result,Err(RailgunError::UserOp(error)) if matches!(*error,kohaku_railgun_paymaster::UserOpError::PoiStatusUnavailable))
    );
    assert!(bundler.ops.lock().unwrap().is_empty());
    assert!(session.poi_pending().is_empty());
}
