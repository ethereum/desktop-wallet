use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::{DynProvider, Provider};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolValue, sol};
use kohaku_tornadocash::{Withdrawal, merkle_tree::MerkleProof};
use kohaku_tornadocash_paymaster::UserOperationPaymasterExt;
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::Bundler,
    signable_user_operation::SignableUserOperation,
    smart_account::{
        SmartAccount,
        simple_7702_smart_account::{Call as UserOpCall, Simple7702SmartAccount},
    },
    user_operation::{UserOperationGasEstimate, UserOperationHash},
};

use super::{TornadoVault, TornadoVaultError, format_eth};
use crate::{
    bundler::BundlerList,
    call::Call,
    vault::{VaultId, tornadocash::TornadoNote},
};

const FEE_ATTEMPTS: usize = 3;

sol! {
    struct TornadoPaymasterData {
        address instance;
        bytes proof;
        bytes32 root;
        bytes32 nullifierHash;
        address recipient;
        uint256 fee;
    }
}

/// What [`quote_paymaster_withdrawal`] spends, and where the remainder goes.
pub struct PaymasterWithdrawalRequest<'a, S: Signer + Send + Sync> {
    pub vault: &'a TornadoVault<S>,
    pub provider: DynProvider,
    pub owner: PrivateKeySigner,
    pub notes: &'a [TornadoNote],
    pub forward_to: Option<Address>,
    /// When set, the remainder after the sponsor fee must be at least this much.
    pub minimum_forward: Option<U256>,
    /// Calls run after the withdrawals. The ETH they send must fit in what the notes leave after
    /// the sponsor fee. Not combined with `forward_to`.
    pub tail_calls: &'a [Call],
    pub chain_id: u64,
    pub bundlers: BundlerList,
}

/// A user operation quoted with placeholder proofs, so a bundler that saw the quote cannot
/// withdraw any note. [`QuotedWithdrawal::submit`] proves the notes and sends it.
pub struct QuotedWithdrawal<'a, S: Signer + Send + Sync> {
    pub fee: U256,
    pub forwarded: Option<U256>,
    pub sum: U256,
    vault: &'a TornadoVault<S>,
    owner: PrivateKeySigner,
    notes: &'a [TornadoNote],
    after_withdrawals: Vec<UserOpCall>,
    paid_out: Option<U256>,
    signable: SignableUserOperation,
    provider: DynProvider,
    bundlers: BundlerList,
}

/// A user operation a bundler has accepted. [`PaymasterWithdrawal::wait`] reads its receipt.
pub struct PaymasterWithdrawal {
    pub hash: UserOperationHash,
    pub fee: U256,
    pub forwarded: Option<U256>,
    paid_out: Option<U256>,
    sum: U256,
    fresh: Address,
    provider: DynProvider,
    bundlers: BundlerList,
}

/// A withdrawal that was included and succeeded.
pub struct IncludedWithdrawal {
    pub transaction: B256,
    block: Option<u64>,
    paid_out: Option<U256>,
    sum: U256,
    fresh: Address,
    provider: DynProvider,
}

/// What a withdrawal cost once the paymaster's refund landed.
pub struct WithdrawalCost {
    pub fee: U256,
    pub change: U256,
}

struct Quoted {
    signable: SignableUserOperation,
    calls: Vec<UserOpCall>,
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
    required: Option<U256>,
    sponsor: &'a TornadoNote,
}

#[derive(Debug, thiserror::Error)]
pub enum UserOpError {
    #[error("no notes to withdraw")]
    Empty,
    #[error("sponsoring pool {0} has no paymaster")]
    MissingPaymaster(String),
    #[error(
        "after a {} ETH sponsor fee the notes leave {} ETH, which does not cover {} ETH",
        format_eth(*.fee),
        format_eth(*.available),
        format_eth(*.required)
    )]
    Short {
        fee: U256,
        available: U256,
        required: U256,
    },
    #[error("forward_to cannot be combined with tail calls")]
    ForwardWithTailCalls,
    #[error(
        "the proven operation needs more gas than the quoted fee covers, so it was not signed; the bundler estimated it with real proofs, so check `balance` before trying again"
    )]
    GasRose,
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
    #[error(
        "the fee and change of {transaction}, which succeeded, could not be measured: {message}"
    )]
    Unmeasured {
        transaction: String,
        message: String,
    },
    #[error(transparent)]
    Vault(#[from] TornadoVaultError),
    #[error(transparent)]
    Paymaster(Box<kohaku_tornadocash_paymaster::PaymasterError>),
    #[error(transparent)]
    SmartAccount(#[from] kohaku_userop_kit::smart_account::SmartAccountError),
    #[error(transparent)]
    Bundler(#[from] kohaku_userop_kit::bundler::BundlerError),
    #[error("signing failed: {0}")]
    Sign(String),
}

impl From<kohaku_tornadocash_paymaster::PaymasterError> for UserOpError {
    fn from(error: kohaku_tornadocash_paymaster::PaymasterError) -> Self {
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
    pub async fn wait(&self) -> Result<IncludedWithdrawal, UserOpError> {
        let receipt = self.bundlers.wait_for_receipt(self.hash).await?;
        let transaction = receipt.receipt.transaction_hash;
        if !receipt.success {
            return Err(UserOpError::Reverted {
                user_op: self.hash.0.to_string(),
                transaction: transaction.to_string(),
            });
        }
        Ok(IncludedWithdrawal {
            transaction,
            block: receipt.receipt.block_number,
            paid_out: self.paid_out,
            sum: self.sum,
            fresh: self.fresh,
            provider: self.provider.clone(),
        })
    }
}

impl IncludedWithdrawal {
    /// Measures the fee and change from the fresh address's balance across the including block.
    ///
    /// Anything else paid to the fresh address in that block would count as change.
    ///
    /// # Errors
    /// Returns [`UserOpError::Unmeasured`] when the balance cannot be read at that block, or
    /// does not add up. The withdrawal itself has still succeeded.
    pub async fn measure(&self) -> Result<WithdrawalCost, UserOpError> {
        let unmeasured = |message: String| UserOpError::Unmeasured {
            transaction: self.transaction.to_string(),
            message,
        };
        let block = self
            .block
            .ok_or_else(|| unmeasured("the receipt has no block number".to_string()))?;
        let balance_at = |number: u64| async move {
            self.provider
                .get_balance(self.fresh)
                .number(number)
                .await
                .map_err(|error| unmeasured(error.to_string()))
        };
        let before = balance_at(block.saturating_sub(1)).await?;
        let after = balance_at(block).await?;
        let (fee, change) = settle(self.sum, self.paid_out, before, after).ok_or_else(|| {
            unmeasured(format!(
                "the fresh address went from {} to {} ETH, which the withdrawal does not explain",
                format_eth(before),
                format_eth(after)
            ))
        })?;
        Ok(WithdrawalCost { fee, change })
    }
}

impl<S: Signer + Send + Sync> QuotedWithdrawal<'_, S> {
    /// Proves every note but the sponsor's, checks the quoted fee still covers the proven
    /// operation, then signs and sends it.
    ///
    /// # Errors
    /// [`UserOpError::GasRose`] when the quoted fee does not cover the proven operation's gas,
    /// and an error when proving, signing, or every bundler fails.
    pub async fn submit(self) -> Result<PaymasterWithdrawal, UserOpError> {
        let QuotedWithdrawal {
            fee,
            forwarded,
            sum,
            vault,
            owner,
            notes,
            after_withdrawals,
            paid_out,
            mut signable,
            provider,
            bundlers,
        } = self;
        let mut calls = proven_withdrawals(vault, notes, owner.address()).await?;
        calls.extend(after_withdrawals);
        signable.user_op.call_data = Simple7702SmartAccount::<DynProvider>::abi_encode_call(&calls);
        let proven = bundlers.estimate_gas(&signable, None).await?;
        if !fits(&mut signable, &proven, fee) {
            return Err(UserOpError::GasRose);
        }
        let signed = signable
            .sign(&owner)
            .await
            .map_err(|error| UserOpError::Sign(error.to_string()))?;
        let hash = bundlers.send_user_operation(&signed).await?;
        Ok(PaymasterWithdrawal {
            hash,
            fee,
            forwarded,
            paid_out: paid_out.or(forwarded),
            sum,
            fresh: owner.address(),
            provider,
            bundlers,
        })
    }
}

/// Quotes one paymaster user operation from `request.notes` without sending it.
///
/// The largest note, which is first, sponsors gas. Its remainder and every other note are paid
/// to the fresh address that signs the operation. `forward_to` adds a final call that sends
/// whatever is left after the sponsor fee; `tail_calls` run after the withdrawals instead.
///
/// # Errors
/// Returns an error when the sponsoring pool has no paymaster, the notes cannot cover
/// `minimum_forward` or the tail calls after the fee, or proving, estimation, or every bundler
/// fails.
pub async fn quote_paymaster_withdrawal<S>(
    request: PaymasterWithdrawalRequest<'_, S>,
) -> Result<QuotedWithdrawal<'_, S>, UserOpError>
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
        tail_calls,
        chain_id,
        bundlers,
    } = request;

    let sponsor = notes.first().ok_or(UserOpError::Empty)?;
    if sponsor.pool.paymaster.is_none() {
        return Err(UserOpError::MissingPaymaster(sponsor.pool.id()));
    }
    if forward_to.is_some() && !tail_calls.is_empty() {
        return Err(UserOpError::ForwardWithTailCalls);
    }
    let fresh = owner.address();
    let (withdrawal, proof) = vault.withdrawal_inputs(sponsor, fresh).await?;
    let mut tails = placeholder_withdrawals(vault, notes, fresh).await?;
    let withdrawals = tails.len();
    tails.extend(tail_calls.iter().map(|call| UserOpCall {
        target: call.target,
        value: call.value,
        data: call.data.clone(),
    }));
    let spend = tail_calls.iter().map(|call| call.value).sum::<U256>();
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
            required: minimum_forward.max((!spend.is_zero()).then_some(spend)),
            sponsor,
        },
        &mut rng,
        withdrawal,
        &proof,
    )
    .await?;
    Ok(QuotedWithdrawal {
        fee: quoted.fee,
        forwarded: quoted.forwarded,
        sum,
        vault,
        owner,
        notes,
        after_withdrawals: quoted.calls[withdrawals..].to_vec(),
        paid_out: (!spend.is_zero()).then_some(spend),
        signable: quoted.signable,
        provider,
        bundlers,
    })
}

/// Raises `op`'s gas limits to `proven` where it needs more, unless that would cost more than
/// `fee`, which the paymaster requires to cover the operation's maximum gas cost.
fn fits(op: &mut SignableUserOperation, proven: &UserOperationGasEstimate, fee: U256) -> bool {
    let mut raised = op.user_op.clone();
    raised.call_gas_limit = raised.call_gas_limit.max(proven.call_gas_limit);
    raised.verification_gas_limit = raised
        .verification_gas_limit
        .max(proven.verification_gas_limit);
    raised.pre_verification_gas = raised.pre_verification_gas.max(proven.pre_verification_gas);
    raised.paymaster_verification_gas_limit = raised
        .paymaster_verification_gas_limit
        .max(proven.paymaster_verification_gas_limit);
    raised.paymaster_post_op_gas_limit = raised
        .paymaster_post_op_gas_limit
        .max(proven.paymaster_post_op_gas_limit);
    let cost = U256::from(raised.total_gas_limit()) * U256::from(raised.max_fee_per_gas);
    if cost > fee {
        return false;
    }
    op.user_op = raised;
    true
}

async fn quote_until_settled(
    input: &QuoteInput<'_>,
    rng: &mut impl rand::CryptoRng,
    withdrawal: Withdrawal,
    proof: &MerkleProof,
) -> Result<Quoted, UserOpError> {
    eprintln!(
        "Quoting the fee and proving the sponsoring {} ETH note.",
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
            .ok_or_else(|| short(fee, U256::ZERO, input.required.unwrap_or(U256::ZERO)))?;
        if let Some(required) = input.required
            && available < required
        {
            return Err(short(fee, available, required));
        }
        if input.forward_to.is_none() || pending_forward == Some(available) {
            return Ok(Quoted {
                signable,
                calls,
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

async fn placeholder_withdrawals<S: Signer + Send + Sync>(
    vault: &TornadoVault<S>,
    notes: &[TornadoNote],
    fresh: Address,
) -> Result<Vec<UserOpCall>, UserOpError> {
    let mut calls = Vec::with_capacity(notes.len().saturating_sub(1));
    for note in notes.iter().skip(1) {
        let (withdrawal, proof) = vault.withdrawal_inputs(note, fresh).await?;
        let call = withdrawal.prove_dummy(&proof).call();
        calls.push(UserOpCall {
            target: call.target,
            value: call.value,
            data: call.input,
        });
    }
    Ok(calls)
}

async fn proven_withdrawals<S: Signer + Send + Sync>(
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
    paymaster_fee(data)
}

fn paymaster_fee(data: &[u8]) -> Result<U256, UserOpError> {
    let decoded = TornadoPaymasterData::abi_decode(data)
        .map_err(|error| UserOpError::FeeDecode(error.to_string()))?;
    Ok(decoded.fee)
}

fn settle(sum: U256, paid_out: Option<U256>, before: U256, after: U256) -> Option<(U256, U256)> {
    let change = after.checked_sub(before)?;
    let fee = sum
        .checked_sub(paid_out.unwrap_or_default())?
        .checked_sub(change)?;
    Some((fee, change))
}

fn short(fee: U256, available: U256, required: U256) -> UserOpError {
    UserOpError::Short {
        fee,
        available,
        required,
    }
}

#[cfg(test)]
mod tests {
    use alloy_sol_types::Eip712Domain;

    use super::*;

    fn gas(pre_verification_gas: u128) -> UserOperationGasEstimate {
        UserOperationGasEstimate {
            pre_verification_gas,
            verification_gas_limit: 100,
            call_gas_limit: 100,
            paymaster_verification_gas_limit: Some(100),
            paymaster_post_op_gas_limit: Some(100),
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 1,
        }
    }

    fn quoted() -> SignableUserOperation {
        UserOperationBuilder::<()>::new(Address::ZERO, Address::ZERO, Eip712Domain::default())
            .with_gas(gas(100))
            .build()
    }

    #[test]
    fn fee_decodes_direct_tornado_paymaster_data() {
        let data = TornadoPaymasterData {
            instance: Address::repeat_byte(1),
            proof: Bytes::from(vec![2; 256]),
            root: B256::repeat_byte(3),
            nullifierHash: B256::repeat_byte(4),
            recipient: Address::repeat_byte(5),
            fee: U256::from(123_456),
        }
        .abi_encode();
        assert_eq!(paymaster_fee(&data).unwrap(), U256::from(123_456));
    }

    #[test]
    fn a_paymaster_refund_counts_as_change_not_fee() {
        let (fee, change) = settle(
            U256::from(200),
            Some(U256::from(123)),
            U256::ZERO,
            U256::from(76),
        )
        .unwrap();
        assert_eq!((fee, change), (U256::from(1), U256::from(76)));
    }

    #[test]
    fn a_proven_limit_above_the_quote_is_adopted_while_the_fee_covers_it() {
        let mut op = quoted();
        assert!(fits(&mut op, &gas(110), U256::from(510)));
        assert_eq!(op.user_op.pre_verification_gas, 110);
    }

    #[test]
    fn a_proven_limit_the_fee_cannot_cover_is_refused() {
        let mut op = quoted();
        assert!(!fits(&mut op, &gas(110), U256::from(509)));
        assert_eq!(op.user_op.pre_verification_gas, 100);
    }
}
