use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::DynProvider;
use alloy_signer::Signer;
use alloy_sol_types::{SolCall, SolValue, sol};
use kohaku_tornadocash::{
    Asset, Withdrawal, merkle_tree::MerkleProof, userop_provider::UserOperationPaymasterExt,
};
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::Bundler,
    signable_user_operation::SignableUserOperation,
    smart_account::simple_7702_smart_account::{Call as UserOpCall, Simple7702SmartAccount},
    user_operation::UserOperationHash,
};

use super::{
    TornadoVault, TornadoVaultError,
    estimation::{FundingBundler, seed_overrides},
    format_amount,
};
use crate::{
    bundler::BundlerList,
    call::Call,
    vault::{VaultId, tornadocash::TornadoNote},
};

const FEE_ATTEMPTS: usize = 3;

sol! {
    function transfer(address to, uint256 amount) external returns (bool);

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
    pub owner: &'a (dyn Signer + Send + Sync),
    pub asset: &'a Asset,
    pub tail_calls: &'a [Call],
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
    asset: &'a Asset,
    funding: &'a alloy_rpc_types_eth::state::StateOverride,
}

#[derive(Debug, thiserror::Error)]
pub enum UserOpError {
    #[error("invalid withdrawal: {0}")]
    Invalid(String),
    #[error("final operation simulation: {0}")]
    Simulation(String),
    #[error("no notes to withdraw")]
    Empty,
    #[error("sponsoring pool {0} has no paymaster")]
    MissingPaymaster(String),
    #[error(
        "after a {fee} sponsor fee the notes leave {available}, which does not cover {required}"
    )]
    Short {
        fee: String,
        available: String,
        required: String,
    },
    #[error("the sponsor fee kept changing while the forward was quoted (last fee {fee})")]
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
/// The first note sponsors gas. Its remainder and every other note are paid
/// to the fresh address that signs the operation. `forward_to` adds a final call that sends
/// the selected asset left after the sponsor fee. Custom calls instead run after withdrawals;
/// forwarding and custom calls are mutually exclusive. An ETH sponsor for an ERC-20 leaves
/// its ETH change on the fresh account. Signing occurs only after unmodified final simulation.
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
        asset,
        tail_calls: custom_calls,
        notes,
        forward_to,
        minimum_forward,
        chain_id,
        bundlers,
    } = request;

    let sponsor = notes.first().ok_or(UserOpError::Empty)?;
    require_eth_sponsor(&sponsor.pool.asset)?;
    if sponsor.pool.paymaster.is_none() {
        return Err(UserOpError::MissingPaymaster(sponsor.pool.id()));
    }
    if notes.iter().any(|note| note.pool.chain_id != chain_id) {
        return Err(UserOpError::Invalid("notes belong to another chain".into()));
    }
    if notes.iter().skip(1).any(|note| &note.pool.asset != asset) {
        return Err(UserOpError::Invalid("mixed withdrawal assets".into()));
    }
    if forward_to.is_some() && !custom_calls.is_empty() {
        return Err(UserOpError::Invalid(
            "forward_to and custom tail calls are mutually exclusive".into(),
        ));
    }
    if super::token_address(asset).is_some()
        && custom_calls.iter().any(|call| !call.value.is_zero())
    {
        return Err(UserOpError::Invalid(
            "ERC-20 tail calls cannot spend ETH".into(),
        ));
    }
    let fresh = owner.address();
    let (withdrawal, proof) = vault.withdrawal_inputs(sponsor, fresh).await?;
    let mut tails = tail_calls(vault, notes, fresh).await?;
    tails.extend(custom_calls.iter().map(|call| UserOpCall {
        target: call.target,
        data: call.data.clone(),
        value: call.value,
    }));
    let funding = seed_overrides(&provider, &sponsor.pool, fresh).await?;
    let sum = notes
        .iter()
        .filter(|note| &note.pool.asset == asset)
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
            asset,
            funding: &funding,
        },
        &mut rng,
        withdrawal,
        &proof,
    )
    .await?;
    // No funding or verifier overrides here: the real proofs must fund and execute every call.
    // Dummy account signatures are accepted by the bundler's estimation path.
    validate_final(&bundlers, &quoted.signable).await?;
    let signed = quoted
        .signable
        .sign(owner)
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

fn require_eth_sponsor(asset: &Asset) -> Result<(), UserOpError> {
    if asset != &Asset::ETH {
        return Err(UserOpError::Invalid(
            "only ETH notes may sponsor withdrawals; ERC-20 paymasters and adapters are not used"
                .into(),
        ));
    }
    Ok(())
}

async fn validate_final(
    bundlers: &BundlerList,
    signable: &SignableUserOperation,
) -> Result<(), UserOpError> {
    let final_gas = bundlers
        .estimate_gas(signable, None)
        .await
        .map_err(|error| UserOpError::Simulation(error.to_string()))?;
    let op = &signable.user_op;
    if final_gas.call_gas_limit > op.call_gas_limit
        || final_gas.verification_gas_limit > op.verification_gas_limit
        || final_gas.pre_verification_gas > op.pre_verification_gas
        || final_gas.paymaster_verification_gas_limit.unwrap_or(0)
            > op.paymaster_verification_gas_limit.unwrap_or(0)
        || final_gas.paymaster_post_op_gas_limit.unwrap_or(0)
            > op.paymaster_post_op_gas_limit.unwrap_or(0)
    {
        return Err(UserOpError::Simulation(
            "gas increased after quoting; retry before signing".into(),
        ));
    }
    Ok(())
}

async fn quote_until_settled(
    input: &QuoteInput<'_>,
    rng: &mut impl rand::CryptoRng,
    withdrawal: Withdrawal,
    proof: &MerkleProof,
) -> Result<Quoted, UserOpError> {
    eprintln!(
        "Proving sponsoring {} {} note.",
        input.sponsor.pool.amount(),
        input.sponsor.pool.symbol().to_uppercase()
    );
    let mut calls = input.tails.to_vec();
    let mut pending_forward = None;
    for pass in 0..FEE_ATTEMPTS {
        if pass > 0 {
            eprintln!("Quoting the fee again so the forward matches it.");
        }
        let signable = quote(input, &calls, withdrawal.clone(), proof, rng).await?;
        let fee = proven_fee(&signable)?;
        let asset_fee = if &input.sponsor.pool.asset == input.asset {
            fee
        } else {
            U256::ZERO
        };
        let available = input.sum.checked_sub(asset_fee).ok_or_else(|| {
            short(
                input.asset,
                fee,
                U256::ZERO,
                input.minimum_forward.unwrap_or(U256::ZERO),
            )
        })?;
        let native_spend = input
            .tails
            .iter()
            .try_fold(U256::ZERO, |sum, call| sum.checked_add(call.value))
            .ok_or_else(|| UserOpError::Invalid("tail-call value overflow".into()))?;
        if super::token_address(input.asset).is_none() && native_spend > available {
            return Err(short(input.asset, fee, available, native_spend));
        }
        if let Some(required) = input.minimum_forward
            && available < required
        {
            return Err(short(input.asset, fee, available, required));
        }
        if input.forward_to.is_none()
            || pending_forward.is_some_and(|pending| {
                pending <= available && pending >= input.minimum_forward.unwrap_or(U256::ZERO)
            })
        {
            return Ok(Quoted {
                signable,
                fee,
                forwarded: input.forward_to.and(pending_forward),
            });
        }
        pending_forward = Some(available);
        calls = forward_calls(input.tails, input.forward_to, available, input.asset);
        if pass + 1 == FEE_ATTEMPTS {
            return Err(UserOpError::Unstable {
                fee: format_amount(fee, input.sponsor.pool.asset.decimals()),
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
    asset: &Asset,
) -> Vec<UserOpCall> {
    let mut calls = tails.to_vec();
    if let Some(target) = forward_to.filter(|_| !available.is_zero()) {
        calls.push(match super::token_address(asset) {
            None => UserOpCall {
                target,
                value: available,
                data: Bytes::new(),
            },
            Some(token) => UserOpCall {
                target: token,
                value: U256::ZERO,
                data: transferCall::new((target, available)).abi_encode().into(),
            },
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
            "Proving {} {} ({}/{})...",
            note.pool.amount(),
            note.pool.symbol().to_uppercase(),
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
    input: &QuoteInput<'_>,
    calls: &[UserOpCall],
    withdrawal: Withdrawal,
    proof: &MerkleProof,
    rng: &mut impl rand::CryptoRng,
) -> Result<SignableUserOperation, UserOpError> {
    let calls = calls.to_vec();
    let builder = UserOperationBuilder::new_with_smart_account(input.account)
        .await?
        .with_call(&calls)
        .with_tornado_paymaster(
            withdrawal,
            proof,
            input.provider,
            &FundingBundler {
                inner: input.bundlers,
                funding: input.funding,
            },
            rng,
        )
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

fn short(asset: &Asset, fee: U256, available: U256, required: U256) -> UserOpError {
    UserOpError::Short {
        fee: format!("{} ETH", format_amount(fee, Asset::ETH.decimals())),
        available: format_amount(available, asset.decimals()),
        required: format_amount(required, asset.decimals()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn core_rejects_erc20_sponsors() {
        assert!(require_eth_sponsor(&Asset::ETH).is_ok());
        for asset in [
            Asset::ETHEREUM_USDC,
            Asset::ETHEREUM_USDT,
            Asset::ETHEREUM_DAI,
            Asset::ETHEREUM_WBTC,
            Asset::SEPOLIA_DAI,
        ] {
            assert!(require_eth_sponsor(&asset).is_err());
        }
    }

    #[test]
    fn token_forward_uses_transfer_with_zero_eth_after_existing_withdrawals() {
        let recipient = Address::repeat_byte(0x42);
        let withdrawal = UserOpCall {
            target: Address::repeat_byte(0x11),
            value: U256::ZERO,
            data: Bytes::from_static(b"withdrawal"),
        };
        let amount = U256::from(100_000_000);
        let calls = forward_calls(
            std::slice::from_ref(&withdrawal),
            Some(recipient),
            amount,
            &Asset::ETHEREUM_USDC,
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].data, withdrawal.data);
        assert_eq!(
            calls[1].target,
            super::super::token_address(&Asset::ETHEREUM_USDC).unwrap()
        );
        assert_eq!(calls[1].value, U256::ZERO);
        let decoded = transferCall::abi_decode(&calls[1].data).unwrap();
        assert_eq!(decoded.to, recipient);
        assert_eq!(decoded.amount, amount);
        let calls = forward_calls(&[], Some(recipient), amount, &Asset::ETH);
        assert_eq!(calls[0].target, recipient);
        assert_eq!(calls[0].value, amount);
        assert!(calls[0].data.is_empty());
    }
}
