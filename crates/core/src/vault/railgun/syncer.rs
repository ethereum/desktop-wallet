//! Saga history and an RPC tip, for both note events and TXID operations.
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::Arc,
    time::Duration,
};

use alloy_consensus::Transaction;
use alloy_primitives::{Address, B256, Bytes, LogData, U64};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::{Filter, Log};
use alloy_sol_types::SolEvent;
use kohaku_railgun::{
    abis::railgun::RailgunSmartWallet as R,
    chain_config::ChainConfig,
    indexer::syncer::{Operation, SyncEvent, SyncerError, TxidSyncer, UtxoSyncer},
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::{RailgunError, Result, decode};
use crate::network::{NetworkEndpoint, http::outbound_http};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    from_block: U64,
    to_block: U64,
    file: String,
    digest: ChunkDigest,
}
#[derive(Clone, Deserialize)]
struct ChunkDigest {
    #[serde(rename = "type")]
    kind: String,
    data: B256,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stream {
    chain_id: U64,
    #[serde(default)]
    tracked_addresses: Vec<Address>,
    chunks: Vec<Chunk>,
    hot_head: Option<Chunk>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    available_protocols: HashMap<String, Stream>,
}

/// Both traits share fetched chunks during one session. Checkpoints belong to
/// the vault; public CDN chunks need no wallet identifiers in their requests.
pub struct RailgunSyncer {
    chain: ChainConfig,
    endpoint: Arc<dyn NetworkEndpoint>,
    provider: DynProvider,
    saga_url: Option<String>,
    client: reqwest::Client,
    manifest: Mutex<Option<Manifest>>,
    cached: Mutex<HashMap<String, Arc<Vec<Value>>>>,
}

impl RailgunSyncer {
    pub fn new(
        chain: ChainConfig,
        endpoint: Arc<dyn NetworkEndpoint>,
        saga_url: Option<&str>,
    ) -> Result<Self> {
        let provider = endpoint.provider().ok_or_else(|| {
            RailgunError::Invalid("Railgun requires an Alloy RPC provider".into())
        })?;
        Ok(Self {
            chain,
            endpoint,
            provider,
            saga_url: saga_url.map(|s| s.trim_end_matches('/').to_string()),
            client: outbound_http().timeout(Duration::from_secs(45)).build()?,
            manifest: Mutex::new(None),
            cached: Mutex::new(HashMap::new()),
        })
    }

    async fn stream(&self, suffix: &str) -> Result<Option<Stream>> {
        let Some(base) = &self.saga_url else {
            return Ok(None);
        };
        let mut manifest = self.manifest.lock().await;
        if manifest.is_none() {
            *manifest = Some(
                self.client
                    .get(format!("{base}/index.json"))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?,
            );
        }
        let stream = manifest
            .as_ref()
            .and_then(|m| {
                m.available_protocols
                    .get(&format!("railgun-{}-{suffix}", self.chain.id))
            })
            .cloned();
        if let Some(s) = &stream
            && (s.chain_id.to::<u64>() != self.chain.id
                || (suffix == "smartwallet"
                    && !s
                        .tracked_addresses
                        .contains(&self.chain.railgun_smart_wallet)))
        {
            return Err(RailgunError::Invalid(
                "Saga stream chain or contract mismatch".into(),
            ));
        }
        Ok(stream)
    }

    async fn chunk(&self, chunk: &Chunk) -> Result<Arc<Vec<Value>>> {
        let mut cache = self.cached.lock().await;
        if let Some(records) = cache.get(&chunk.file) {
            return Ok(records.clone());
        }
        let base = self
            .saga_url
            .as_ref()
            .ok_or_else(|| RailgunError::Invalid("missing Saga URL".into()))?;
        let bytes = self
            .client
            .get(format!("{base}/{}", chunk.file))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if chunk.digest.kind != "sha256"
            || B256::from_slice(&Sha256::digest(&bytes)) != chunk.digest.data
        {
            return Err(RailgunError::Invalid(format!(
                "Saga digest mismatch: {}",
                chunk.file
            )));
        }
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_ref())
            .take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)?;
        if raw.len() > 256 * 1024 * 1024 {
            return Err(RailgunError::Invalid(
                "Saga chunk exceeds size limit".into(),
            ));
        }
        let mut rows = Vec::new();
        let mut previous = None;
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let row: Value = serde_json::from_slice(line)?;
            let block = quantity(&row, "blockNumber")?;
            let second = quantity(
                &row,
                if row.get("entity").is_some() {
                    "transactionIndex"
                } else {
                    "logIndex"
                },
            )?;
            let third = if row.get("entity").is_some() {
                quantity(&row, "opIndex")?
            } else {
                0
            };
            let key = (block, second, third);
            if block < chunk.from_block.to()
                || block >= chunk.to_block.to()
                || previous.is_some_and(|p| p >= key)
            {
                return Err(RailgunError::Invalid(
                    "Saga chunk has out-of-range, duplicate or unordered records".into(),
                ));
            }
            previous = Some(key);
            rows.push(row);
        }
        // Keep just the last two chunks (one per stream), not the entire history.
        if cache.len() >= 2 {
            cache.clear();
        }
        let rows = Arc::new(rows);
        cache.insert(chunk.file.clone(), rows.clone());
        Ok(rows)
    }

    async fn records(&self, suffix: &str, from: u64, to: u64) -> Result<(Vec<Value>, u64)> {
        let Some(stream) = self.stream(suffix).await? else {
            return Ok((Vec::new(), from));
        };
        let mut chunks: Vec<_> = stream.chunks.into_iter().chain(stream.hot_head).collect();
        chunks.sort_by_key(|c| c.from_block);
        let mut cursor = from;
        let mut rows = Vec::new();
        for chunk in chunks {
            if chunk.to_block.to::<u64>() <= cursor {
                continue;
            }
            if chunk.from_block.to::<u64>() > cursor || cursor > to {
                break;
            }
            let end = chunk.to_block.to::<u64>().min(to.saturating_add(1));
            for row in self.chunk(&chunk).await?.iter() {
                let block = quantity(row, "blockNumber")?;
                if block >= cursor && block < end {
                    rows.push(row.clone());
                }
            }
            cursor = end;
        }
        Ok((rows, cursor))
    }

    async fn logs(&self, from: u64, to: u64) -> Result<Vec<Log>> {
        if from > to {
            return Ok(Vec::new());
        }
        let (rows, cursor) = match self.records("smartwallet", from, to).await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!("Saga unavailable ({e}); reading Railgun logs from RPC");
                (Vec::new(), from)
            }
        };
        let mut logs = rows
            .iter()
            .map(|v| canonical_log(v, self.chain.railgun_smart_wallet))
            .collect::<Result<Vec<_>>>()?;
        if cursor <= to {
            logs.extend(
                self.endpoint
                    .logs_in_range(
                        &Filter::new().address(self.chain.railgun_smart_wallet),
                        cursor..=to,
                        self.endpoint.event_block_range(),
                    )
                    .await?,
            );
        }
        Ok(logs)
    }

    async fn operations(&self, from: u64, to: u64) -> Result<Vec<Operation>> {
        let from = from.max(self.chain.deployment_block);
        if from > to {
            return Ok(Vec::new());
        }
        let (rows, cursor) = match self.records("ops", from, to).await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!("Saga operations unavailable ({e}); reading calldata from RPC");
                (Vec::new(), from)
            }
        };
        let mut operations = rows.iter().map(operation).collect::<Result<Vec<_>>>()?;
        if !operations.is_empty() {
            // Saga omits hasUnshield. A final commitment absent from that
            // block's emitted UTXO leaves is the unshield commitment.
            let logs = self.logs(from, cursor.saturating_sub(1).min(to)).await?;
            let mut leaves = HashSet::new();
            for log in logs {
                let block = log
                    .block_number
                    .ok_or_else(|| RailgunError::Invalid("log without block".into()))?;
                for event in decode::decode(log)? {
                    match event {
                        SyncEvent::Transact(e, _) => {
                            leaves.insert((block, e.hash));
                        }
                        SyncEvent::Legacy(e, _) => {
                            leaves.insert((block, e.hash));
                        }
                        _ => {}
                    }
                }
            }
            for op in &mut operations {
                op.has_unshield = op
                    .commitment_hashes
                    .last()
                    .is_some_and(|h| !leaves.contains(&(op.block_number, *h)));
            }
        }
        if cursor <= to {
            operations.extend(self.rpc_operations(cursor, to).await?);
        }
        Ok(operations)
    }

    async fn rpc_operations(&self, from: u64, to: u64) -> Result<Vec<Operation>> {
        let logs = self
            .endpoint
            .logs_in_range(
                &Filter::new().address(self.chain.railgun_smart_wallet),
                from..=to,
                self.endpoint.event_block_range(),
            )
            .await?;
        let mut hashes = Vec::new();
        for log in &logs {
            if log.topic0() == Some(&R::Nullified::SIGNATURE_HASH) {
                let hash = log.transaction_hash.ok_or_else(|| {
                    RailgunError::Invalid("nullifier log without transaction hash".into())
                })?;
                if !hashes.contains(&hash) {
                    hashes.push(hash);
                }
            } else if log.topic0() == Some(&decode::Nullifiers::SIGNATURE_HASH) {
                return Err(RailgunError::Invalid(
                    "legacy Railgun operation history needs Saga coverage".into(),
                ));
            }
        }
        let mut out = Vec::new();
        for hash in hashes {
            let tx = self
                .provider
                .get_transaction_by_hash(hash)
                .await
                .map_err(|e| RailgunError::Invalid(e.to_string()))?
                .ok_or_else(|| RailgunError::Invalid(format!("missing transaction {hash}")))?;
            let input = tx.inner.input();
            let relevant: Vec<_> = logs
                .iter()
                .filter(|l| l.transaction_hash == Some(hash))
                .collect();
            out.extend(super::rpc_operations::from_calldata(
                input,
                &relevant,
                &self.chain,
            )?);
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl UtxoSyncer for RailgunSyncer {
    async fn latest_block(&self) -> std::result::Result<u64, SyncerError> {
        self.endpoint.block_height().await.map_err(SyncerError::new)
    }
    async fn sync(&self, from: u64, to: u64) -> std::result::Result<Vec<SyncEvent>, SyncerError> {
        let mut events = Vec::new();
        for log in self
            .logs(from.max(self.chain.deployment_block), to)
            .await
            .map_err(SyncerError::new)?
        {
            events.extend(decode::decode(log).map_err(SyncerError::new)?);
        }
        Ok(events)
    }
}
#[async_trait::async_trait]
impl TxidSyncer for RailgunSyncer {
    async fn latest_block(&self) -> std::result::Result<u64, SyncerError> {
        self.endpoint.block_height().await.map_err(SyncerError::new)
    }
    async fn sync(&self, from: u64, to: u64) -> std::result::Result<Vec<Operation>, SyncerError> {
        self.operations(from, to).await.map_err(SyncerError::new)
    }
}

fn quantity(row: &Value, key: &str) -> Result<u64> {
    let text = row[key]
        .as_str()
        .ok_or_else(|| RailgunError::Invalid(format!("missing Saga quantity {key}")))?;
    u64::from_str_radix(
        text.strip_prefix("0x")
            .ok_or_else(|| RailgunError::Invalid(format!("invalid Saga quantity {key}")))?,
        16,
    )
    .map_err(|e| RailgunError::Invalid(e.to_string()))
}
fn canonical_log(row: &Value, contract: Address) -> Result<Log> {
    let address: Address = serde_json::from_value(row["contractAddress"].clone())?;
    if address != contract {
        return Err(RailgunError::Invalid(
            "Saga event from unexpected contract".into(),
        ));
    }
    let topics: Vec<B256> = serde_json::from_value(row["topics"].clone())?;
    let data: Bytes = serde_json::from_value(row["data"].clone())?;
    let data = LogData::new(topics, data)
        .ok_or_else(|| RailgunError::Invalid("invalid Saga log topics".into()))?;
    Ok(Log {
        inner: alloy_primitives::Log { address, data },
        block_number: Some(quantity(row, "blockNumber")?),
        log_index: Some(quantity(row, "logIndex")?),
        ..Default::default()
    })
}
fn operation(row: &Value) -> Result<Operation> {
    if row["entity"] != "railgun-operation" {
        return Err(RailgunError::Invalid("unexpected Saga entity".into()));
    }
    let number =
        |key| u32::try_from(quantity(row, key)?).map_err(|e| RailgunError::Invalid(e.to_string()));
    let tree = number("utxoTreeOut")?;
    let start = number("utxoBatchStartPositionOut")?;
    let (tree, start) = if (tree, start) == (99999, 99999) {
        (tree, start)
    } else {
        decode::position(
            alloy_primitives::U256::from(tree),
            alloy_primitives::U256::from(start),
            0,
        )?
    };
    Ok(Operation {
        block_number: quantity(row, "blockNumber")?,
        nullifiers: serde_json::from_value(row["nullifiers"].clone())?,
        commitment_hashes: serde_json::from_value(row["commitments"].clone())?,
        bound_params_hash: serde_json::from_value(row["boundParamsHash"].clone())?,
        utxo_tree_in: number("utxoTreeIn")?,
        utxo_tree_out: tree,
        utxo_out_start_index: start,
        has_unshield: false,
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

    #[test]
    fn public_historical_saga_events_decode_all_published_generations() {
        let rows: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/historical-events.json")).unwrap();
        assert_eq!(rows.len(), 7);
        let mut legacy = 0;
        let mut shield = 0;
        let mut transact = 0;
        let mut nullifiers = 0;
        for row in rows {
            for event in decode::decode(
                canonical_log(&row, ChainConfig::mainnet().railgun_smart_wallet).unwrap(),
            )
            .unwrap()
            {
                match event {
                    SyncEvent::Legacy(_, _) => legacy += 1,
                    SyncEvent::Shield(_, _) => shield += 1,
                    SyncEvent::Transact(_, _) => transact += 1,
                    SyncEvent::Nullified(_, _) => nullifiers += 1,
                }
            }
        }
        assert!(legacy > 0 && shield > 0 && transact > 0 && nullifiers > 0);
    }

    #[test]
    fn operations_keep_sentinel_positions_and_full_precision_hashes() {
        let row = serde_json::json!({"entity":"railgun-operation","blockNumber":"0x100","transactionIndex":"0x0","opIndex":"0x0","nullifiers":["0x1"],"commitments":["0x2"],"boundParamsHash":"0x3","utxoTreeIn":"0x1","utxoTreeOut":"0x1869f","utxoBatchStartPositionOut":"0x1869f"});
        let op = operation(&row).unwrap();
        assert_eq!(op.utxo_tree_out, 99999);
        assert_eq!(op.utxo_out_start_index, 99999);
        assert_eq!(op.bound_params_hash, U256::from(3));
        assert_eq!(op.nullifiers, vec![U256::from(1)]);
        let mut overflow = row.clone();
        overflow["utxoTreeOut"] = Value::String("0x2".into());
        overflow["utxoBatchStartPositionOut"] = Value::String("0x10005".into());
        let normalized = operation(&overflow).unwrap();
        assert_eq!(
            (normalized.utxo_tree_out, normalized.utxo_out_start_index),
            (3, 5)
        );
        let mut bad = row;
        bad["utxoTreeIn"] = Value::String("0x100000000".into());
        assert!(operation(&bad).is_err());
    }

    #[tokio::test]
    async fn a_gap_never_claims_saga_coverage_past_the_gap() {
        let syncer = RailgunSyncer::new(
            ChainConfig::mainnet(),
            mocked_provider(&Asserter::new()),
            Some("https://unused.test"),
        )
        .unwrap();
        let chunk = |from, to, file: &str| Chunk {
            from_block: U64::from(from),
            to_block: U64::from(to),
            file: file.into(),
            digest: ChunkDigest {
                kind: "sha256".into(),
                data: B256::ZERO,
            },
        };
        let stream = Stream {
            chain_id: U64::from(1),
            tracked_addresses: vec![syncer.chain.railgun_smart_wallet],
            chunks: vec![chunk(10, 20, "one"), chunk(30, 40, "two")],
            hot_head: None,
        };
        *syncer.manifest.lock().await = Some(Manifest {
            available_protocols: HashMap::from([("railgun-1-smartwallet".into(), stream)]),
        });
        syncer.cached.lock().await.insert(
            "one".into(),
            Arc::new(vec![serde_json::json!({"blockNumber":"0xf"})]),
        );
        let (rows, cursor) = syncer.records("smartwallet", 10, 39).await.unwrap();
        assert_eq!(cursor, 20);
        assert_eq!(rows.len(), 1);
        let (rows, cursor) = syncer.records("smartwallet", 21, 39).await.unwrap();
        assert_eq!(cursor, 21);
        assert!(rows.is_empty());
        let (_, cursor) = syncer.records("smartwallet", 15, 17).await.unwrap();
        assert_eq!(cursor, 18);
    }

    /// Full public sample verification is opt-in, with no funded keys or live
    /// RPC/PPOI traffic. See docs/railgun.md for downloading sample chunks.
    #[test]
    #[ignore = "requires EDW_RAILGUN_SAGA_FIXTURE_DIR public chunk downloads"]
    fn full_public_chunk_decoding() {
        let Ok(dir) = std::env::var("EDW_RAILGUN_SAGA_FIXTURE_DIR") else {
            eprintln!(
                "Skipping optional public-chunk check; set EDW_RAILGUN_SAGA_FIXTURE_DIR to run it."
            );
            return;
        };
        let file =
            std::fs::File::open(std::path::Path::new(&dir).join("edw-saga-logs.gz")).unwrap();
        let mut raw = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut raw)
            .unwrap();
        let mut count = 0;
        for line in raw.lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            count += decode::decode(
                canonical_log(&row, ChainConfig::mainnet().railgun_smart_wallet).unwrap(),
            )
            .unwrap()
            .len();
        }
        assert!(count > 1000);
    }
}
