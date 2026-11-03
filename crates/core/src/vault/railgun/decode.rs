//! All Railgun event generations. Older commitments still occupy tree leaves,
//! even though this wallet only decrypts current notes.
use alloy_primitives::{B256, U256};
use alloy_rpc_types_eth::Log;
use alloy_sol_types::{SolEvent, sol};
use kohaku_railgun::{
    abis::railgun::{RailgunSmartWallet as R, ShieldCiphertext},
    indexer::syncer::{LegacyCommitment, Nullified, Shield, SyncEvent},
};

use super::{RailgunError, Result};

sol! {
    struct LegacyToken { uint8 tokenType; address tokenAddress; uint256 tokenSubID; }
    struct V20Preimage { bytes32 npk; LegacyToken token; uint120 value; }
    struct V20Ciphertext { bytes32[3] encryptedBundle; bytes32 shieldKey; }
    event GeneratedCommitmentBatch(uint256 treeNumber, uint256 startPosition, LegacyPreimage[] commitments, uint256[2][] encryptedRandom);
    struct LegacyPreimage { uint256 npk; LegacyToken token; uint120 value; }
    event CommitmentBatch(uint256 treeNumber, uint256 startPosition, uint256[] hash, LegacyCiphertext[] ciphertext);
    struct LegacyCiphertext { uint256[4] ciphertext; uint256[2] ephemeralKeys; uint256[] memo; }
    event Nullifiers(uint256 treeNumber, uint256[] nullifier);
    event ShieldV20(uint256 treeNumber, uint256 startPosition, V20Preimage[] commitments, V20Ciphertext[] shieldCiphertext);
}

pub(super) fn position(tree: U256, start: U256, index: usize) -> Result<(u32, u32)> {
    let absolute = tree
        .checked_mul(U256::from(65_536))
        .and_then(|v| v.checked_add(start))
        .and_then(|v| v.checked_add(U256::from(index)))
        .ok_or_else(|| RailgunError::Invalid("tree position overflow".into()))?;
    let tree = u32::try_from(absolute / U256::from(65_536))
        .map_err(|_| RailgunError::Invalid("tree number overflow".into()))?;
    Ok((tree, (absolute % U256::from(65_536)).to::<u32>()))
}

#[allow(clippy::too_many_lines)]
pub(super) fn decode(log: Log) -> Result<Vec<SyncEvent>> {
    let topic = log
        .topic0()
        .copied()
        .ok_or_else(|| RailgunError::Invalid("event without topic".into()))?;
    let block = log
        .block_number
        .ok_or_else(|| RailgunError::Invalid("event without block".into()))?;
    if topic == GeneratedCommitmentBatch::SIGNATURE_HASH {
        let event = GeneratedCommitmentBatch::decode_log(&log.inner)?;
        return event
            .commitments
            .iter()
            .enumerate()
            .map(|(i, note)| {
                let (tree_number, leaf_index) = position(event.treeNumber, event.startPosition, i)?;
                let shield = Shield {
                    tree_number,
                    leaf_index,
                    npk: note.npk,
                    token: token(&note.token)?,
                    value: U256::from(note.value),
                    ciphertext: ShieldCiphertext {
                        encryptedBundle: [B256::ZERO; 3],
                        shieldKey: B256::ZERO,
                    }
                    .into(),
                    shield_key: [0; 32],
                    hash: None,
                };
                Ok(SyncEvent::Legacy(
                    LegacyCommitment {
                        tree_number,
                        leaf_index,
                        hash: shield.hash().into(),
                    },
                    block,
                ))
            })
            .collect();
    }
    if topic == CommitmentBatch::SIGNATURE_HASH {
        let event = CommitmentBatch::decode_log(&log.inner)?;
        return event
            .hash
            .iter()
            .enumerate()
            .map(|(i, hash)| {
                let (tree_number, leaf_index) = position(event.treeNumber, event.startPosition, i)?;
                Ok(SyncEvent::Legacy(
                    LegacyCommitment {
                        tree_number,
                        leaf_index,
                        hash: *hash,
                    },
                    block,
                ))
            })
            .collect();
    }
    if topic == Nullifiers::SIGNATURE_HASH {
        let event = Nullifiers::decode_log(&log.inner)?;
        let tree_number = u32::try_from(event.treeNumber)
            .map_err(|_| RailgunError::Invalid("tree number overflow".into()))?;
        return Ok(event
            .nullifier
            .iter()
            .map(|n| {
                SyncEvent::Nullified(
                    Nullified {
                        tree_number,
                        nullifier: B256::from(n.to_be_bytes()),
                    },
                    block,
                )
            })
            .collect());
    }
    // The old Shield has the same tuple layout as ShieldV20, but its name is
    // Shield. Substitute only the signature after matching the actual topic.
    let old_shield = alloy_primitives::keccak256(b"Shield(uint256,uint256,(bytes32,(uint8,address,uint256),uint120)[],(bytes32[3],bytes32)[])");
    if topic == old_shield {
        let mut inner = log.inner.clone();
        inner.data.topics_mut()[0] = ShieldV20::SIGNATURE_HASH;
        let event = ShieldV20::decode_log(&inner)?;
        if event.commitments.len() != event.shieldCiphertext.len() {
            return Err(RailgunError::Invalid(
                "shield ciphertext count mismatch".into(),
            ));
        }
        return event
            .commitments
            .iter()
            .zip(&event.shieldCiphertext)
            .enumerate()
            .map(|(i, (note, cipher))| {
                let (tree_number, leaf_index) = position(event.treeNumber, event.startPosition, i)?;
                Ok(SyncEvent::Shield(
                    Shield {
                        tree_number,
                        leaf_index,
                        npk: note.npk.into(),
                        token: token(&note.token)?,
                        value: U256::from(note.value),
                        ciphertext: ShieldCiphertext {
                            encryptedBundle: cipher.encryptedBundle,
                            shieldKey: cipher.shieldKey,
                        }
                        .into(),
                        shield_key: cipher.shieldKey.into(),
                        hash: None,
                    },
                    block,
                ))
            })
            .collect();
    }
    // Unknown administrative events are irrelevant. Known note events must
    // decode successfully; silently skipping a malformed note would lose funds.
    if [
        R::Shield::SIGNATURE_HASH,
        R::Transact::SIGNATURE_HASH,
        R::Nullified::SIGNATURE_HASH,
        R::Unshield::SIGNATURE_HASH,
    ]
    .contains(&topic)
    {
        return kohaku_railgun_sync::log_to_sync_events(log)
            .map_err(|e| RailgunError::Invalid(e.to_string()));
    }
    Ok(Vec::new())
}

fn token(token: &LegacyToken) -> Result<kohaku_railgun::caip::AssetId> {
    use kohaku_railgun::caip::AssetId;
    match token.tokenType {
        0 => Ok(AssetId::Erc20(token.tokenAddress)),
        1 => Ok(AssetId::Erc721(token.tokenAddress, token.tokenSubID)),
        2 => Ok(AssetId::Erc1155(token.tokenAddress, token.tokenSubID)),
        _ => Err(RailgunError::Invalid("unknown Railgun token type".into())),
    }
}
