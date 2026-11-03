use alloy_primitives::{B256, Bytes, U64};
use alloy_transport::mock::Asserter;
use kohaku_railgun::{
    abis::railgun::{RailgunSmartWallet, RelayAdapt},
    poi::types::BlindedCommitmentType,
};
use railgun_userop_kit::{
    bundler::{BundlerError, GasPrice},
    signable_user_operation::SignableUserOperation,
    user_operation::{UserOperationHash, UserOperationReceipt},
};

use super::*;
use crate::{database::memory::MemoryDatabase, test_support::mocked_provider};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

struct ReceiptBundler {
    pending: PendingOperation,
    transaction: B256,
}

struct FailingBundler;

#[async_trait::async_trait]
impl Bundler for FailingBundler {
    async fn gas_price(&self) -> std::result::Result<Option<GasPrice>, BundlerError> {
        Ok(None)
    }
    async fn estimate_gas(
        &self,
        _: &SignableUserOperation,
        _: Option<&alloy_rpc_types_eth::state::StateOverride>,
    ) -> std::result::Result<UserOperationGasEstimate, BundlerError> {
        Err(BundlerError::Other("not expected".into()))
    }
    async fn send_user_operation(
        &self,
        _: &SignedUserOperation,
    ) -> std::result::Result<UserOperationHash, BundlerError> {
        Err(BundlerError::Other("connection lost".into()))
    }
    async fn receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<Option<UserOperationReceipt>, BundlerError> {
        Ok(None)
    }
    async fn wait_for_receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<UserOperationReceipt, BundlerError> {
        Err(BundlerError::Timeout)
    }
}

#[async_trait::async_trait]
impl Bundler for ReceiptBundler {
    async fn gas_price(&self) -> std::result::Result<Option<GasPrice>, BundlerError> {
        Ok(None)
    }
    async fn estimate_gas(
        &self,
        _: &SignableUserOperation,
        _: Option<&alloy_rpc_types_eth::state::StateOverride>,
    ) -> std::result::Result<UserOperationGasEstimate, BundlerError> {
        Err(BundlerError::Other("not expected".into()))
    }
    async fn send_user_operation(
        &self,
        _: &SignedUserOperation,
    ) -> std::result::Result<UserOperationHash, BundlerError> {
        Err(BundlerError::Other(
            "must not resubmit an included operation".into(),
        ))
    }
    async fn receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<Option<UserOperationReceipt>, BundlerError> {
        Ok(Some(UserOperationReceipt {
            entry_point: self.pending.signed.entry_point,
            user_op_hash: self.pending.signed.hash(1).0,
            sender: self.pending.signed.user_op.sender,
            nonce: self.pending.signed.user_op.nonce,
            actual_gas_used: U256::ZERO,
            actual_gas_cost: U256::ZERO,
            success: true,
            reason: None,
            logs: Vec::new(),
            receipt: alloy_rpc_types_eth::TransactionReceipt {
                inner: alloy_consensus::ReceiptWithBloom::default(),
                transaction_hash: self.transaction,
                transaction_index: Some(0),
                block_hash: Some(B256::repeat_byte(8)),
                block_number: Some(1),
                gas_used: 0,
                effective_gas_price: 0,
                blob_gas_used: None,
                blob_gas_price: None,
                from: Address::ZERO,
                to: None,
                contract_address: None,
            },
        }))
    }
    async fn wait_for_receipt(
        &self,
        _: UserOperationHash,
    ) -> std::result::Result<UserOperationReceipt, BundlerError> {
        Err(BundlerError::Timeout)
    }
}

fn vault(db: Arc<dyn Database>, asserter: &Asserter) -> RailgunVault {
    RailgunVault::open(
        &Mnemonic::parse(PHRASE).unwrap(),
        0,
        1,
        db,
        mocked_provider(asserter),
        None,
    )
    .unwrap()
}

#[test]
fn unshield_gross_up_delivers_exact_net_and_is_minimal() {
    for bps in [0, 1, 25, 50, 500, 5000] {
        for net in 1..10_000 {
            let gross = gross_up(net, bps).unwrap();
            let net_of = |gross| gross - gross * u128::from(bps) / 10_000;
            assert_eq!(net_of(gross), net);
            assert!(net_of(gross - 1) < net);
        }
    }
    assert!(gross_up((1 << 120) - 1, 25).is_err());
    assert!(gross_up(1, 10_000).is_err());
    assert_eq!(gross_up(9975, 25).unwrap(), 9999);
}

#[test]
fn native_shield_wraps_into_the_profiles_railgun_account() {
    let vault = vault(Arc::new(MemoryDatabase::new()), &Asserter::new());
    let calls = vault.deposit(&AssetId::Native, U256::from(42)).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].target, vault.chain.relay_adapt_contract);
    assert_eq!(calls[0].value, U256::from(42));
    let relay = RelayAdapt::multicallCall::abi_decode(&calls[0].data).unwrap();
    assert!(relay._requireSuccess);
    assert_eq!(relay._calls.len(), 2);
    assert_eq!(
        RelayAdapt::wrapBaseCall::abi_decode(&relay._calls[0].data)
            .unwrap()
            ._amount,
        U256::from(42)
    );
    let shield = RelayAdapt::shieldCall::abi_decode(&relay._calls[1].data).unwrap();
    assert_eq!(
        shield._shieldRequests[0].preimage.token.tokenAddress,
        vault.chain.wrapped_base_token
    );
    assert_eq!(
        U256::from(shield._shieldRequests[0].preimage.value),
        U256::from(42)
    );
}

#[test]
fn token_shield_approves_exact_amount_before_shield() {
    let vault = vault(Arc::new(MemoryDatabase::new()), &Asserter::new());
    let token = Address::repeat_byte(0x11);
    let calls = vault
        .deposit(&AssetId::Erc20(token), U256::from(123))
        .unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].target, token);
    let approval = approveCall::abi_decode(&calls[0].data).unwrap();
    assert_eq!(approval.spender, vault.chain.railgun_smart_wallet);
    assert_eq!(approval.amount, U256::from(123));
    let shield = RailgunSmartWallet::shieldCall::abi_decode(&calls[1].data).unwrap();
    assert_eq!(shield._shieldRequests[0].preimage.token.tokenAddress, token);
    assert!(vault.deposit(&AssetId::Native, U256::ZERO).is_err());
    assert!(
        vault
            .deposit(&AssetId::Native, U256::from(1u128 << 120))
            .is_err()
    );
}

#[test]
fn unwrap_precedes_tails_and_uses_the_exact_net_amount() {
    let weth = Address::repeat_byte(0x11);
    let tail = Call::new(
        Address::repeat_byte(0x22),
        Bytes::from_static(&[0xab]),
        U256::from(42),
    );
    let calls = execution_calls(weth, weth, 42, true, vec![tail.clone()]).unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].target, weth);
    assert_eq!(
        withdrawCall::abi_decode(&calls[0].data).unwrap().amount,
        U256::from(42)
    );
    assert_eq!(calls[1].target, tail.target);
    assert_eq!(calls[1].data, tail.data);
    assert_eq!(calls[1].value, tail.value);
    assert!(execution_calls(weth, tail.target, 42, true, vec![]).is_err());
    assert!(
        execution_calls(weth, tail.target, 42, false, vec![])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn only_locally_valid_notes_are_spendable_and_weth_is_not_native_eth() {
    let token = Address::repeat_byte(0x11);
    let entry = |amount, status| NoteEntry {
        asset: RailgunAsset::Erc20(token),
        poi_status: status,
        amount,
        tree_number: 0,
        leaf_index: 0,
        blinded_commitment: String::new(),
        commitment_type: BlindedCommitmentType::Transact,
        memo: String::new(),
    };
    let balance = note_balance(
        vec![
            entry(10, Some(PoiStatus::Valid)),
            entry(20, Some(PoiStatus::Unavailable)),
            entry(30, None),
        ],
        token,
    );
    assert_eq!(
        balance,
        Balance {
            spendable: U256::from(10),
            pending: U256::from(50)
        }
    );
    assert_eq!(
        note_balance(vec![entry(10, Some(PoiStatus::Valid))], Address::ZERO),
        Balance::default()
    );
}

#[tokio::test]
async fn checkpoint_roundtrips_pending_operation_and_rejects_another_profile() {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    let asserter = Asserter::new();
    asserter.push_success(&U64::from(1));
    asserter.push_success(&U64::from(1));
    asserter.push_success(&U64::from(1));
    let vault = vault(db.clone(), &asserter);
    let pending = PendingOperation {
        signed: SignedUserOperation::default(),
        fee: U256::from(42),
        fresh_index: 7,
    };
    {
        let mut session = vault.session().await.unwrap();
        session.pending = Some(pending.clone());
        session.persist().await.unwrap();
    }
    {
        let mut session = vault.session().await.unwrap();
        let recovered = session.pending().unwrap();
        assert_eq!(recovered.fresh_index, 7);
        assert_eq!(recovered.fee, pending.fee);
        assert_eq!(
            serde_json::to_value(&recovered.signed).unwrap(),
            serde_json::to_value(&pending.signed).unwrap()
        );
        let owner = AccountSigner::from_bytes(&B256::repeat_byte(1)).unwrap();
        assert!(matches!(
            session
                .quote(
                    Movement::Transfer {
                        token: Address::ZERO,
                        amount: U256::from(1),
                        recipient: vault.address(),
                        memo: String::new()
                    },
                    &owner,
                    &FailingBundler
                )
                .await,
            Err(RailgunError::Pending(_))
        ));
        assert!(session.submit_pending(&FailingBundler).await.is_err());
        assert!(session.pending().is_some());
    }
    let other = RailgunVault::open(
        &Mnemonic::parse(PHRASE).unwrap(),
        1,
        1,
        db,
        mocked_provider(&asserter),
        None,
    )
    .unwrap();
    assert!(matches!(
        other.session().await,
        Err(RailgunError::Invalid(_))
    ));
}

#[tokio::test]
async fn a_receipt_found_after_restart_clears_the_journal_without_sending_again() {
    let asserter = Asserter::new();
    asserter.push_success(&U64::from(1));
    asserter.push_success(&U64::from(1));
    let vault = vault(Arc::new(MemoryDatabase::new()), &asserter);
    let pending = PendingOperation {
        signed: SignedUserOperation::default(),
        fee: U256::from(42),
        fresh_index: 7,
    };
    let tx = B256::repeat_byte(9);
    {
        let mut session = vault.session().await.unwrap();
        session.pending = Some(pending.clone());
        session.persist().await.unwrap();
    }
    let mut session = vault.session().await.unwrap();
    let bundler = ReceiptBundler {
        pending: pending.clone(),
        transaction: tx,
    };
    let (hash, transaction) = session.submit_pending(&bundler).await.unwrap();
    assert_eq!(hash, pending.signed.hash(1).0.to_string());
    assert_eq!(transaction, tx.to_string());
    assert!(session.pending().is_none());
    assert!(!session.observed);
    let stored: StoredState =
        serde_json::from_slice(&vault.db.get(CHECKPOINT).await.unwrap().unwrap()).unwrap();
    assert!(stored.pending.is_none());
}
