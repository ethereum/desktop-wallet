use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::DynProvider;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolValue, sol};
use kohaku_tornadocash::{
    Withdrawal, merkle_tree::MerkleProof, userop_provider::UserOperationPaymasterExt,
};
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::Bundler,
    signable_user_operation::SignableUserOperation,
    smart_account::simple_7702_smart_account::{Call as UserOpCall, Simple7702SmartAccount},
    user_operation::UserOperationHash,
};

use super::{TornadoVault, TornadoVaultError, format_eth};
use crate::{
    bundler::BundlerList,
    vault::{VaultId, tornadocash::TornadoNote},
};

const FEE_ATTEMPTS: usize = 3;

sol! {
    struct PaymasterData {
        address adapter;
        bytes adapterData;
    }

    struct TornadoAdapterData {
        bytes proof;
        bytes32 root;
        bytes32 nullifierHash;
        address recipient;
        address relayer;
        uint256 fee;
        uint256 refund;
    }
}

/// What [`submit_paymaster_withdrawal`] spends, and where the remainder goes.
pub struct PaymasterWithdrawalRequest<'a, S: Signer + Send + Sync> {
    pub vault: &'a TornadoVault<S>,
    pub provider: DynProvider,
    pub owner: PrivateKeySigner,
    pub notes: &'a [TornadoNote],
    pub forward_to: Option<Address>,
    /// When set, the remainder after the sponsor fee must be at least this much.
    pub minimum_forward: Option<U256>,
    pub chain_id: u64,
    pub bundlers: BundlerList,
}

/// A user operation a bundler has accepted. [`PaymasterWithdrawal::wait`] reads its receipt.
pub struct PaymasterWithdrawal {
    pub hash: UserOperationHash,
    pub fee: U256,
    pub forwarded: Option<U256>,
    bundlers: BundlerList,
}

struct Quoted {
    signable: SignableUserOperation,
    fee: U256,
    forwarded: Option<U256>,
}

struct QuoteInput<'a> {
    account: &'a Simple7702SmartAccount<DynProvider>,
    provider: &'a DynProvider,
    bundlers: &'a BundlerList,
    tails: &'a [UserOpCall],
    sum: U256,
    forward_to: Option<Address>,
    minimum_forward: Option<U256>,
    sponsor: &'a TornadoNote,
}

#[derive(Debug, thiserror::Error)]
pub enum UserOpError {
    #[error("no notes to withdraw")]
    Empty,
    #[error("sponsoring pool {0} has no paymaster")]
    MissingPaymaster(String),
    #[error(
        "after a {fee} ETH sponsor fee the notes leave {available} ETH, which does not cover {required} ETH"
    )]
    Short {
        fee: String,
        available: String,
        required: String,
    },
    #[error("the sponsor fee kept changing while the forward was quoted (last fee {fee} ETH)")]
    Unstable { fee: String },
    #[error("paymaster data has no fee")]
    FeeMissing,
    #[error("paymaster data: {0}")]
    FeeDecode(String),
    #[error("user operation {user_op} was included in {transaction} and reverted")]
    Reverted {
        user_op: String,
        transaction: String,
    },
    #[error(transparent)]
    Vault(#[from] TornadoVaultError),
    #[error(transparent)]
    Paymaster(Box<kohaku_tornadocash::userop_provider::TornadoPaymasterError>),
    #[error(transparent)]
    SmartAccount(#[from] kohaku_userop_kit::smart_account::SmartAccountError),
    #[error(transparent)]
    Bundler(#[from] kohaku_userop_kit::bundler::BundlerError),
    #[error("signing failed: {0}")]
    Sign(String),
}

impl From<kohaku_tornadocash::userop_provider::TornadoPaymasterError> for UserOpError {
    fn from(error: kohaku_tornadocash::userop_provider::TornadoPaymasterError) -> Self {
        Self::Paymaster(Box::new(error))
    }
}

impl PaymasterWithdrawal {
    /// The bundler that accepted the operation.
    #[must_use]
    pub fn bundler(&self) -> Option<String> {
        self.bundlers.accepted_url()
    }

    /// Polls the accepting bundler until the operation is included.
    ///
    /// # Errors
    /// Returns an error when the receipt times out, or the included operation reverted.
    pub async fn wait(&self) -> Result<alloy_primitives::B256, UserOpError> {
        let receipt = self.bundlers.wait_for_receipt(self.hash).await?;
        if !receipt.success {
            return Err(UserOpError::Reverted {
                user_op: self.hash.0.to_string(),
                transaction: receipt.receipt.transaction_hash.to_string(),
            });
        }
        Ok(receipt.receipt.transaction_hash)
    }
}

/// Builds one paymaster user operation from `request.notes` and sends it.
///
/// The largest note, which is first, sponsors gas. Its remainder and every other note are paid
/// to the fresh address that signs the operation. `forward_to` adds a final call that sends
/// whatever is left after the sponsor fee.
///
/// # Errors
/// Returns an error when the sponsoring pool has no paymaster, the notes cannot cover
/// `minimum_forward` after the fee, or proving, estimation, or every bundler fails.
pub async fn submit_paymaster_withdrawal<S>(
    request: PaymasterWithdrawalRequest<'_, S>,
) -> Result<PaymasterWithdrawal, UserOpError>
where
    S: Signer + Send + Sync,
{
    let PaymasterWithdrawalRequest {
        vault,
        provider,
        owner,
        notes,
        forward_to,
        minimum_forward,
        chain_id,
        bundlers,
    } = request;

    let sponsor = notes.first().ok_or(UserOpError::Empty)?;
    if sponsor.pool.paymaster.is_none() {
        return Err(UserOpError::MissingPaymaster(sponsor.pool.id()));
    }
    let fresh = owner.address();
    let (withdrawal, proof) = vault.withdrawal_inputs(sponsor, fresh).await?;
    let tails = tail_calls(vault, notes, fresh).await?;
    let sum = notes
        .iter()
        .map(|note| U256::from(note.pool.amount_wei))
        .sum::<U256>();
    let account = Simple7702SmartAccount::new(provider.clone(), fresh, chain_id);
    let mut rng = rand::rng();
    let quoted = quote_until_settled(
        &QuoteInput {
            account: &account,
            provider: &provider,
            bundlers: &bundlers,
            tails: &tails,
            sum,
            forward_to,
            minimum_forward,
            sponsor,
        },
        &mut rng,
        withdrawal,
        &proof,
    )
    .await?;
    let signed = quoted
        .signable
        .sign(&owner)
        .await
        .map_err(|error| UserOpError::Sign(error.to_string()))?;
    let hash = bundlers.send_user_operation(&signed).await?;
    Ok(PaymasterWithdrawal {
        hash,
        fee: quoted.fee,
        forwarded: quoted.forwarded,
        bundlers,
    })
}

async fn quote_until_settled(
    input: &QuoteInput<'_>,
    rng: &mut impl rand::CryptoRng,
    withdrawal: Withdrawal,
    proof: &MerkleProof,
) -> Result<Quoted, UserOpError> {
    eprintln!(
        "Proving the sponsoring {} ETH note. The fee is quoted, then proved again.",
        format_eth(U256::from(input.sponsor.pool.amount_wei))
    );
    let mut calls = input.tails.to_vec();
    let mut pending_forward = None;
    for pass in 0..FEE_ATTEMPTS {
        if pass > 0 {
            eprintln!("Quoting the fee again so the forward matches it.");
        }
        let signable = quote(
            input.account,
            &calls,
            withdrawal.clone(),
            proof,
            input.provider,
            input.bundlers,
            rng,
        )
        .await?;
        let fee = proven_fee(&signable)?;
        let available = input
            .sum
            .checked_sub(fee)
            .ok_or_else(|| short(fee, U256::ZERO, input.minimum_forward.unwrap_or(U256::ZERO)))?;
        if let Some(required) = input.minimum_forward
            && available < required
        {
            return Err(short(fee, available, required));
        }
        if input.forward_to.is_none() || pending_forward == Some(available) {
            return Ok(Quoted {
                signable,
                fee,
                forwarded: input.forward_to.map(|_| available),
            });
        }
        pending_forward = Some(available);
        calls = forward_calls(input.tails, input.forward_to, available);
        if pass + 1 == FEE_ATTEMPTS {
            return Err(UserOpError::Unstable {
                fee: format_eth(fee),
            });
        }
    }
    Err(UserOpError::Unstable {
        fee: "0".to_string(),
    })
}

fn forward_calls(
    tails: &[UserOpCall],
    forward_to: Option<Address>,
    available: U256,
) -> Vec<UserOpCall> {
    let mut calls = tails.to_vec();
    if let Some(target) = forward_to.filter(|_| !available.is_zero()) {
        calls.push(UserOpCall {
            target,
            value: available,
            data: Bytes::new(),
        });
    }
    calls
}

async fn tail_calls<S: Signer + Send + Sync>(
    vault: &TornadoVault<S>,
    notes: &[TornadoNote],
    fresh: Address,
) -> Result<Vec<UserOpCall>, UserOpError> {
    let tails = notes.len().saturating_sub(1);
    if tails > 0 {
        eprintln!("Proving {tails} other notes.");
    }
    let mut calls = Vec::with_capacity(tails);
    for (index, note) in notes.iter().skip(1).enumerate() {
        eprintln!(
            "Proving {} ETH ({}/{})...",
            format_eth(U256::from(note.pool.amount_wei)),
            index + 1,
            tails
        );
        let call = vault.withdraw(note, &VaultId::Address(fresh)).await?;
        calls.push(UserOpCall {
            target: call.target,
            value: call.value,
            data: call.data,
        });
    }
    Ok(calls)
}

async fn quote(
    account: &Simple7702SmartAccount<DynProvider>,
    calls: &[UserOpCall],
    withdrawal: Withdrawal,
    proof: &MerkleProof,
    provider: &DynProvider,
    bundlers: &BundlerList,
    rng: &mut impl rand::CryptoRng,
) -> Result<SignableUserOperation, UserOpError> {
    let calls = calls.to_vec();
    let builder = UserOperationBuilder::new_with_smart_account(account)
        .await?
        .with_call(&calls)
        .with_tornado_paymaster(withdrawal, proof, provider, bundlers, rng)
        .await?;
    Ok(builder.build())
}

fn proven_fee(op: &SignableUserOperation) -> Result<U256, UserOpError> {
    let Some(data) = op.user_op.paymaster_data.as_deref() else {
        return Err(UserOpError::FeeMissing);
    };
    let outer = PaymasterData::abi_decode(data)
        .map_err(|error| UserOpError::FeeDecode(error.to_string()))?;
    let inner = TornadoAdapterData::abi_decode(outer.adapterData.as_ref())
        .map_err(|error| UserOpError::FeeDecode(error.to_string()))?;
    Ok(inner.fee)
}

fn short(fee: U256, available: U256, required: U256) -> UserOpError {
    UserOpError::Short {
        fee: format_eth(fee),
        available: format_eth(available),
        required: format_eth(required),
    }
}
