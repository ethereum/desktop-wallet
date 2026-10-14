use std::{ops::Range, sync::Arc};

use alloy_rpc_types_eth::Filter;
use kohaku_tornadocash::{
    Pool,
    syncer::{Snapshot, SyncEvent, Syncer, SyncerError},
};
use tracing::{info, warn};

use crate::network::{DEFAULT_EVENT_BLOCK_RANGE, NetworkEndpoint, NetworkEndpointError};

/// A syncer that reads a pool's events from a [`NetworkEndpoint`].
pub struct NetworkEndpointSyncer {
    provider: Arc<dyn NetworkEndpoint>,
}

impl NetworkEndpointSyncer {
    pub fn new(provider: Arc<dyn NetworkEndpoint>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl Syncer for NetworkEndpointSyncer {
    async fn sync_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, SyncerError> {
        self.sync(pool, from_block, to_block)
            .await
            .map_err(SyncerError::other)
    }
}

impl NetworkEndpointSyncer {
    async fn sync(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, NetworkEndpointError> {
        let range = self.block_range(pool, from_block, to_block).await?;
        if range.is_empty() {
            return Ok(Snapshot::new(pool.clone(), range, Vec::new()));
        }
        info!("Syncing {pool} from {} to {}", range.start, range.end);

        //? `logs_in_range` takes an inclusive range, `range` is half-open.
        let filter = Filter::new().address(pool.address);
        let logs = self
            .provider
            .logs_in_range(
                &filter,
                range.start..=range.end - 1,
                DEFAULT_EVENT_BLOCK_RANGE,
            )
            .await?;

        let mut events = Vec::with_capacity(logs.len());
        for log in logs {
            let Some(decoded) = SyncEvent::try_from_log(&log) else {
                warn!("Unknown event with topic {:?}", log.topic0());
                continue;
            };
            events.push(decoded);
        }

        Ok(Snapshot::new(pool.clone(), range, events))
    }

    /// Clamps the requested range to the blocks this syncer can serve: the pool's deployment
    /// block through the chain's latest block.
    async fn block_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Range<u64>, NetworkEndpointError> {
        let from = from_block.max(pool.deployed_block);
        //? `block_height` is an inclusive block number, `to_block` an exclusive bound.
        let latest = self.provider.block_height().await?.saturating_add(1);
        let to = to_block.min(latest).max(from);

        Ok(from..to)
    }
}
