use std::{ops::Range, sync::Arc, time::Duration};

use alloy_rpc_types_eth::Filter;
use kohaku_tornadocash::{
    Pool,
    syncer::{Snapshot, SyncEvent, Syncer, SyncerError, saga::SagaSyncer},
};
use tracing::{info, warn};

use crate::network::{
    DEFAULT_EVENT_BLOCK_RANGE, NetworkEndpoint, NetworkEndpointError, http::outbound_http,
};

/// Default Saga CDN. `EDW_SAGA_URL` replaces this base; the chain suffix is applied after that.
pub const SAGA_CDN_BASE: &str = "https://saga.gordosoluciones.xyz";

const SAGA_URL_ENV: &str = "EDW_SAGA_URL";

/// How long a Saga request may take before the syncer treats the CDN as down.
const SAGA_TIMEOUT: Duration = Duration::from_secs(30);

/// Saga first, then the unsynced tip from `fallback`.
///
/// A missing pool and a dead CDN are both a primary error, and the fallback takes the whole
/// range. A primary snapshot that ends early is kept, and the fallback is asked only for the
/// blocks after it.
pub struct ChainedSyncer<P, F> {
    primary: Option<P>,
    fallback: F,
}

/// A syncer that reads a pool's events from a [`NetworkEndpoint`].
pub struct NetworkEndpointSyncer {
    provider: Arc<dyn NetworkEndpoint>,
}

impl ChainedSyncer<SagaSyncer, NetworkEndpointSyncer> {
    /// Saga for mainnet and Sepolia, and the endpoint for every chain.
    ///
    /// # Errors
    /// Returns an error if the Saga HTTP client cannot be built.
    pub fn tornado(
        chain_id: u64,
        endpoint: Arc<dyn NetworkEndpoint>,
    ) -> Result<Self, NetworkEndpointError> {
        let primary = match saga_base_url(chain_id) {
            Some(url) => {
                let client = outbound_http()
                    .timeout(SAGA_TIMEOUT)
                    .build()
                    .map_err(|error| NetworkEndpointError::Backend(Box::new(error)))?;
                Some(SagaSyncer::with_client(client, &url))
            }
            None => None,
        };
        Ok(Self {
            primary,
            fallback: NetworkEndpointSyncer::new(endpoint),
        })
    }
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

#[async_trait::async_trait]
impl<P, F> Syncer for ChainedSyncer<P, F>
where
    P: Syncer,
    F: Syncer,
{
    async fn sync_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, SyncerError> {
        let Some(primary) = &self.primary else {
            return self.fallback.sync_range(pool, from_block, to_block).await;
        };
        chain_range(primary, &self.fallback, pool, from_block, to_block).await
    }
}

/// The Saga base for `chain_id`, or `None` when this chain has no CDN.
///
/// Mainnet uses the base as-is. Sepolia uses `{base}/sepolia`.
#[must_use]
pub fn saga_base_url(chain_id: u64) -> Option<String> {
    saga_base_url_with(chain_id, std::env::var(SAGA_URL_ENV).ok())
}

fn saga_base_url_with(chain_id: u64, override_url: Option<String>) -> Option<String> {
    let base = override_url.unwrap_or_else(|| SAGA_CDN_BASE.to_string());
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    match chain_id {
        1 => Some(base.to_string()),
        11_155_111 => Some(format!("{base}/sepolia")),
        _ => None,
    }
}

/// Joins `primary` and `fallback` over `from_block..to_block`.
///
/// A fallback failure on the tail is returned as-is, so a partial range is not committed.
async fn chain_range<P, F>(
    primary: &P,
    fallback: &F,
    pool: &Pool,
    from_block: u64,
    to_block: u64,
) -> Result<Snapshot, SyncerError>
where
    P: Syncer,
    F: Syncer,
{
    let saga = match primary.sync_range(pool, from_block, to_block).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            info!("Saga sync failed ({error}); reading the range from the endpoint");
            return fallback.sync_range(pool, from_block, to_block).await;
        }
    };

    if saga.range.start > from_block {
        info!(
            "Saga snapshot starts at {}, after the requested {from_block}; reading the range from the endpoint",
            saga.range.start
        );
        return fallback.sync_range(pool, from_block, to_block).await;
    }
    if saga.range.end >= to_block {
        return Ok(saga);
    }

    info!(
        "Saga covers {}..{}; reading the tip from the endpoint",
        saga.range.start, saga.range.end
    );
    let tail = fallback.sync_range(pool, saga.range.end, to_block).await?;
    if tail.range.start != saga.range.end {
        info!(
            "Endpoint tail starts at {}, not {}; reading the whole range from the endpoint",
            tail.range.start, saga.range.end
        );
        return fallback.sync_range(pool, from_block, to_block).await;
    }

    let mut events = saga.events;
    events.extend(tail.events);
    let end = tail.range.end.max(saga.range.end);
    Ok(Snapshot::new(saga.pool, saga.range.start..end, events))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Script {
        answers: Mutex<Vec<Result<Snapshot, String>>>,
        calls: Mutex<Vec<(u64, u64)>>,
    }

    impl Script {
        fn new(answers: Vec<Result<Snapshot, String>>) -> Self {
            Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(u64, u64)> {
            self.calls.lock().expect("script lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl Syncer for Script {
        async fn sync_range(
            &self,
            pool: &Pool,
            from_block: u64,
            to_block: u64,
        ) -> Result<Snapshot, SyncerError> {
            self.calls
                .lock()
                .expect("script lock")
                .push((from_block, to_block));
            let _ = pool;
            self.answers
                .lock()
                .expect("script lock")
                .remove(0)
                .map_err(|error| SyncerError::other(std::io::Error::other(error)))
        }
    }

    fn snap(start: u64, end: u64) -> Snapshot {
        Snapshot::new(Pool::ETHEREUM_ETHER_01, start..end, Vec::new())
    }

    #[tokio::test]
    async fn a_full_saga_snapshot_does_not_ask_the_endpoint() {
        let saga = Script::new(vec![Ok(snap(10, 40))]);
        let rpc = Script::new(vec![]);
        let joined = chain_range(&saga, &rpc, &Pool::ETHEREUM_ETHER_01, 10, 40)
            .await
            .expect("saga covered the range");
        assert_eq!(joined.range, 10..40);
        assert!(rpc.calls().is_empty());
    }

    #[tokio::test]
    async fn a_saga_error_reads_the_whole_range_from_the_endpoint() {
        let saga = Script::new(vec![Err("down".into())]);
        let rpc = Script::new(vec![Ok(snap(10, 40))]);
        let joined = chain_range(&saga, &rpc, &Pool::ETHEREUM_ETHER_01, 10, 40)
            .await
            .expect("endpoint covers the range");
        assert_eq!(joined.range, 10..40);
        assert_eq!(rpc.calls(), vec![(10, 40)]);
    }

    #[tokio::test]
    async fn a_saga_snapshot_that_ends_early_is_followed_by_the_endpoint_tail() {
        let saga = Script::new(vec![Ok(snap(10, 25))]);
        let rpc = Script::new(vec![Ok(snap(25, 40))]);
        let joined = chain_range(&saga, &rpc, &Pool::ETHEREUM_ETHER_01, 10, 40)
            .await
            .expect("tail abuts");
        assert_eq!(joined.range, 10..40);
        assert_eq!(rpc.calls(), vec![(25, 40)]);
    }

    #[tokio::test]
    async fn a_tail_that_does_not_abut_is_replaced_by_a_full_endpoint_read() {
        let saga = Script::new(vec![Ok(snap(10, 25))]);
        let rpc = Script::new(vec![Ok(snap(30, 40)), Ok(snap(10, 40))]);
        let joined = chain_range(&saga, &rpc, &Pool::ETHEREUM_ETHER_01, 10, 40)
            .await
            .expect("full read replaces the hole");
        assert_eq!(joined.range, 10..40);
        assert_eq!(rpc.calls(), vec![(25, 40), (10, 40)]);
    }

    #[tokio::test]
    async fn a_saga_snapshot_that_starts_late_is_discarded() {
        let saga = Script::new(vec![Ok(snap(15, 40))]);
        let rpc = Script::new(vec![Ok(snap(10, 40))]);
        let joined = chain_range(&saga, &rpc, &Pool::ETHEREUM_ETHER_01, 10, 40)
            .await
            .expect("endpoint covers the hole at the start");
        assert_eq!(joined.range, 10..40);
        assert_eq!(rpc.calls(), vec![(10, 40)]);
    }

    #[test]
    fn saga_urls_follow_the_chain() {
        assert_eq!(saga_base_url_with(1, None).as_deref(), Some(SAGA_CDN_BASE));
        assert_eq!(
            saga_base_url_with(11_155_111, None).as_deref(),
            Some("https://saga.gordosoluciones.xyz/sepolia")
        );
        assert_eq!(saga_base_url_with(31_337, None), None);
        assert_eq!(
            saga_base_url_with(1, Some("https://cdn.example/".into())).as_deref(),
            Some("https://cdn.example")
        );
        assert_eq!(
            saga_base_url_with(11_155_111, Some("https://cdn.example".into())).as_deref(),
            Some("https://cdn.example/sepolia")
        );
    }
}
