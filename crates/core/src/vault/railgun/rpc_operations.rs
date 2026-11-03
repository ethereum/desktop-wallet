//! Reconstruct the RPC tip from transaction calldata and matching chain logs.
//! ERC-4337 fee transactions live in paymasterAndData, not account callData.
use std::collections::BTreeMap;

use alloy_primitives::U256;
use alloy_rpc_types_eth::Log;
use alloy_sol_types::{SolCall, SolEvent, SolValue, sol};
use kohaku_railgun::{
    abis::railgun::{RailgunSmartWallet as R, Transaction, UnshieldType},
    chain_config::ChainConfig,
    indexer::syncer::Operation,
};

use super::{RailgunError, Result};

sol! {
    struct PackedOp {
        address sender; uint256 nonce; bytes initCode; bytes callData;
        bytes32 accountGasLimits; uint256 preVerificationGas; bytes32 gasFees;
        bytes paymasterAndData; bytes signature;
    }
    struct AggregatedOps { PackedOp[] userOps; address aggregator; bytes signature; }
    function handleOps(PackedOp[] ops, address beneficiary);
    function handleAggregatedOps(AggregatedOps[] opsPerAggregator, address beneficiary);
    struct PaymasterEnvelope { address adapter; bytes adapterData; }
    struct FeeAdapterData { bytes16 random; address asset; uint120 value; Transaction[] transactions; }
}

#[allow(clippy::too_many_lines)]
pub(super) fn from_calldata(
    input: &[u8],
    logs: &[&Log],
    chain: &ChainConfig,
) -> Result<Vec<Operation>> {
    let mut transactions = Vec::new();
    // The Railgun call may be nested inside RelayAdapt, a multisend, or an
    // account's execution bytes. Only candidates matching emitted nullifiers
    // and all output leaves can enter the TXID tree.
    for offset in 0..input.len().saturating_sub(3) {
        let bytes = &input[offset..];
        if bytes.starts_with(&R::transactCall::SELECTOR)
            && let Ok(call) = R::transactCall::abi_decode(bytes)
        {
            transactions.extend(call._transactions);
        }
        let ops = if bytes.starts_with(&handleOpsCall::SELECTOR) {
            handleOpsCall::abi_decode(bytes).ok().map(|c| c.ops)
        } else if bytes.starts_with(&handleAggregatedOpsCall::SELECTOR) {
            handleAggregatedOpsCall::abi_decode(bytes).ok().map(|c| {
                c.opsPerAggregator
                    .into_iter()
                    .flat_map(|g| g.userOps)
                    .collect()
            })
        } else {
            None
        };
        for op in ops.into_iter().flatten() {
            let data = op.paymasterAndData;
            if data.len() < 52 || chain.privacy_paymaster.is_none_or(|p| data[..20] != p[..]) {
                continue;
            }
            let Ok(envelope) = PaymasterEnvelope::abi_decode(&data[52..]) else {
                continue;
            };
            if Some(envelope.adapter) != chain.railgun_fee_adapter {
                continue;
            }
            let adapter = FeeAdapterData::abi_decode(&envelope.adapterData)?;
            transactions.extend(adapter.transactions);
        }
    }
    let mut operations = BTreeMap::new();
    for transaction in transactions {
        let Some(log) = logs.iter().find(|l| {
            R::Nullified::decode_log(&l.inner).is_ok_and(|n| {
                n.nullifier == transaction.nullifiers
                    && n.treeNumber == transaction.boundParams.treeNumber
            })
        }) else {
            continue;
        };
        let has_unshield = !matches!(transaction.boundParams.unshield, UnshieldType::NONE);
        let output_count = transaction
            .commitments
            .len()
            .checked_sub(usize::from(has_unshield))
            .ok_or_else(|| RailgunError::Invalid("unshield without commitment".into()))?;
        let (tree, start) = if output_count == 0 {
            (99999, 99999)
        } else {
            let event = logs
                .iter()
                .filter_map(|l| R::Transact::decode_log(&l.inner).ok())
                .find(|e| e.hash == transaction.commitments[..output_count])
                .ok_or_else(|| {
                    RailgunError::Invalid("transact calldata has no matching output event".into())
                })?;
            super::decode::position(event.treeNumber, event.startPosition, 0)?
        };
        let operation = Operation {
            block_number: log
                .block_number
                .ok_or_else(|| RailgunError::Invalid("log without block".into()))?,
            nullifiers: transaction
                .nullifiers
                .iter()
                .map(|h| U256::from_be_bytes(h.0))
                .collect(),
            commitment_hashes: transaction
                .commitments
                .iter()
                .map(|h| U256::from_be_bytes(h.0))
                .collect(),
            bound_params_hash: transaction.boundParams.hash(),
            utxo_tree_in: u32::from(transaction.boundParams.treeNumber),
            utxo_tree_out: tree,
            utxo_out_start_index: start,
            has_unshield,
        };
        let index = log
            .log_index
            .ok_or_else(|| RailgunError::Invalid("log without index".into()))?;
        if let Some(previous) = operations.insert(index, operation.clone())
            && (previous.bound_params_hash != operation.bound_params_hash
                || previous.commitment_hashes != operation.commitment_hashes)
        {
            return Err(RailgunError::Invalid(
                "ambiguous transaction calldata".into(),
            ));
        }
    }
    let count = logs
        .iter()
        .filter(|l| l.topic0() == Some(&R::Nullified::SIGNATURE_HASH))
        .count();
    if operations.len() != count {
        return Err(RailgunError::Invalid("cannot reconstruct every Railgun operation from calldata; refusing to advance TXID checkpoint".into()));
    }
    Ok(operations.into_values().collect())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, Bytes};
    use kohaku_railgun::abis::railgun::{
        BoundParams, CommitmentPreimage, G1Point, G2Point, SnarkProof,
    };

    use super::*;

    fn transaction(unshield: bool) -> Transaction {
        Transaction {
            proof: SnarkProof {
                a: G1Point {
                    x: U256::ZERO,
                    y: U256::ZERO,
                },
                b: G2Point {
                    x: [U256::ZERO; 2],
                    y: [U256::ZERO; 2],
                },
                c: G1Point {
                    x: U256::ZERO,
                    y: U256::ZERO,
                },
            },
            merkleRoot: B256::ZERO,
            nullifiers: vec![B256::repeat_byte(1)],
            commitments: if unshield {
                vec![B256::repeat_byte(2), B256::repeat_byte(3)]
            } else {
                vec![B256::repeat_byte(2)]
            },
            boundParams: BoundParams::new(
                0,
                0,
                if unshield {
                    UnshieldType::NORMAL
                } else {
                    UnshieldType::NONE
                },
                1,
                Address::ZERO,
                &[0; 32],
                vec![],
            ),
            unshieldPreimage: CommitmentPreimage::default(),
        }
    }
    fn logs(transaction: &Transaction) -> Vec<Log> {
        let nullified = R::Nullified {
            treeNumber: 0,
            nullifier: transaction.nullifiers.clone(),
        };
        let transact = R::Transact {
            treeNumber: U256::from(1),
            startPosition: U256::from(42),
            hash: vec![transaction.commitments[0]],
            ciphertext: vec![],
        };
        vec![
            Log {
                inner: alloy_primitives::Log {
                    address: ChainConfig::mainnet().railgun_smart_wallet,
                    data: nullified.encode_log_data(),
                },
                block_number: Some(20_000_000),
                log_index: Some(1),
                ..Default::default()
            },
            Log {
                inner: alloy_primitives::Log {
                    address: ChainConfig::mainnet().railgun_smart_wallet,
                    data: transact.encode_log_data(),
                },
                block_number: Some(20_000_000),
                log_index: Some(2),
                ..Default::default()
            },
        ]
    }
    #[test]
    fn nested_transact_matches_logs_and_preserves_unshield_flag() {
        for unshield in [false, true] {
            let transaction = transaction(unshield);
            let logs = logs(&transaction);
            let call = R::transactCall {
                _transactions: vec![transaction.clone()],
            }
            .abi_encode();
            let input = [vec![0xab; 17], call].concat();
            let ops = from_calldata(
                &input,
                &logs.iter().collect::<Vec<_>>(),
                &ChainConfig::mainnet(),
            )
            .unwrap();
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].has_unshield, unshield);
            assert_eq!(ops[0].utxo_tree_out, 1);
            assert_eq!(ops[0].utxo_out_start_index, 42);
            assert_eq!(ops[0].bound_params_hash, transaction.boundParams.hash());
        }
    }
    #[test]
    fn privacy_paymaster_payload_is_reconstructed_without_transact_selector() {
        let chain = ChainConfig::mainnet();
        let transaction = transaction(true);
        let logs = logs(&transaction);
        let data = FeeAdapterData {
            random: alloy_primitives::FixedBytes::ZERO,
            asset: chain.wrapped_base_token,
            value: alloy_primitives::aliases::U120::ZERO,
            transactions: vec![transaction],
        }
        .abi_encode();
        let envelope = PaymasterEnvelope {
            adapter: chain.railgun_fee_adapter.unwrap(),
            adapterData: data.into(),
        }
        .abi_encode();
        let paymaster_data = [
            chain.privacy_paymaster.unwrap().as_slice(),
            &[0; 32],
            &envelope,
        ]
        .concat();
        let op = PackedOp {
            sender: Address::ZERO,
            nonce: U256::ZERO,
            initCode: Bytes::new(),
            callData: Bytes::new(),
            accountGasLimits: B256::ZERO,
            preVerificationGas: U256::ZERO,
            gasFees: B256::ZERO,
            paymasterAndData: paymaster_data.into(),
            signature: Bytes::new(),
        };
        let input = handleOpsCall {
            ops: vec![op],
            beneficiary: Address::ZERO,
        }
        .abi_encode();
        let ops = from_calldata(&input, &logs.iter().collect::<Vec<_>>(), &chain).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(ops[0].has_unshield);
    }
    #[test]
    fn absent_calldata_or_missing_output_log_does_not_advance_the_index() {
        let transaction = transaction(false);
        let logs = logs(&transaction);
        assert!(
            from_calldata(
                &[],
                &logs.iter().collect::<Vec<_>>(),
                &ChainConfig::mainnet()
            )
            .is_err()
        );
        let input = R::transactCall {
            _transactions: vec![transaction],
        }
        .abi_encode();
        assert!(from_calldata(&input, &[&logs[0]], &ChainConfig::mainnet()).is_err());
    }
}
