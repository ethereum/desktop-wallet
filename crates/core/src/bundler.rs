use std::{sync::Arc, time::Duration};

use kohaku_userop_kit::{
    bundler::{Bundler, BundlerError, pimlico::PimlicoBundler},
    signed_user_operation::SignedUserOperation,
    user_operation::{UserOperationGasEstimate, UserOperationHash, UserOperationReceipt},
};
use reqwest::Url;

use crate::network::http::outbound_http;

const BUNDLER_TIMEOUT: Duration = Duration::from_mins(3);
const PIMLICO_RPC: &str = "https://public.pimlico.io/v2";

/// Bundlers tried in order. A receipt stays on the one that accepted the send.
pub struct BundlerList {
    endpoints: Vec<Endpoint>,
    chain_id: u64,
    accepted: std::sync::Mutex<Option<usize>>,
}

struct Endpoint {
    url: String,
    bundler: Arc<dyn Bundler + Send + Sync>,
}

#[derive(Debug, thiserror::Error)]
pub enum BundlerListError {
    #[error("EDW_BUNDLER_URLS is empty")]
    Empty,
    #[error("invalid bundler url `{url}`: {message}")]
    Url { url: String, message: String },
    #[error("no bundler for chain {chain_id}; set EDW_BUNDLER_URLS")]
    Unset { chain_id: u64 },
    #[error("bundler http client: {0}")]
    Client(String),
}

impl BundlerList {
    /// One Pimlico client per URL, sharing the outbound HTTP client.
    ///
    /// # Errors
    /// Returns an error when `urls` is empty or the HTTP client cannot be built.
    pub fn from_urls(chain_id: u64, urls: &[Url]) -> Result<Self, BundlerListError> {
        if urls.is_empty() {
            return Err(BundlerListError::Empty);
        }
        let client = outbound_http()
            .timeout(BUNDLER_TIMEOUT)
            .build()
            .map_err(|error| BundlerListError::Client(error.to_string()))?;
        let endpoints = urls
            .iter()
            .map(|url| Endpoint {
                url: url.to_string(),
                bundler: Arc::new(PimlicoBundler::with_client(client.clone(), url.clone())),
            })
            .collect();
        Ok(Self {
            endpoints,
            chain_id,
            accepted: std::sync::Mutex::new(None),
        })
    }

    /// The URL that will be tried first.
    #[must_use]
    pub fn first_url(&self) -> Option<&str> {
        self.endpoints.first().map(|endpoint| endpoint.url.as_str())
    }

    /// The URL that accepted `eth_sendUserOperation`, once one has.
    #[must_use]
    pub fn accepted_url(&self) -> Option<String> {
        let index = self.accepted_index().ok().flatten()?;
        self.endpoints
            .get(index)
            .map(|endpoint| endpoint.url.clone())
    }

    fn accepted_index(&self) -> Result<Option<usize>, BundlerError> {
        let guard = self.accepted.lock().map_err(|error| {
            BundlerError::Other(format!("bundler list lock poisoned: {error}").into())
        })?;
        Ok(*guard)
    }

    fn record(&self, index: usize) -> Result<(), BundlerError> {
        let mut guard = self.accepted.lock().map_err(|error| {
            BundlerError::Other(format!("bundler list lock poisoned: {error}").into())
        })?;
        *guard = Some(index);
        Ok(())
    }
}

#[async_trait::async_trait]
impl Bundler for BundlerList {
    async fn estimate_gas(
        &self,
        op: &kohaku_userop_kit::signable_user_operation::SignableUserOperation,
        state_override: Option<&alloy_rpc_types_eth::state::StateOverride>,
    ) -> Result<UserOperationGasEstimate, BundlerError> {
        let mut failures = Vec::new();
        for endpoint in &self.endpoints {
            match endpoint.bundler.estimate_gas(op, state_override).await {
                Ok(estimate) => return Ok(estimate),
                Err(error) => failures.push(format!("{}: {error}", endpoint.url)),
            }
        }
        Err(failed(&failures))
    }

    async fn send_user_operation(
        &self,
        op: &SignedUserOperation,
    ) -> Result<UserOperationHash, BundlerError> {
        let mut failures = Vec::new();
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            match endpoint.bundler.send_user_operation(op).await {
                Ok(hash) => {
                    self.record(index)?;
                    return Ok(hash);
                }
                Err(error) if already_known(&error) => {
                    self.record(index)?;
                    return Ok(op.hash(self.chain_id));
                }
                Err(error) => failures.push(format!("{}: {error}", endpoint.url)),
            }
        }
        Err(failed(&failures))
    }

    async fn wait_for_receipt(
        &self,
        hash: UserOperationHash,
    ) -> Result<UserOperationReceipt, BundlerError> {
        let Some(index) = self.accepted_index()? else {
            return Err(BundlerError::Other(
                "user operation was not accepted by a bundler".into(),
            ));
        };
        let Some(endpoint) = self.endpoints.get(index) else {
            return Err(BundlerError::Other(
                "accepted bundler is no longer in the list".into(),
            ));
        };
        endpoint.bundler.wait_for_receipt(hash).await
    }
}

/// URLs for `chain_id`. `EDW_BUNDLER_URLS` replaces the Pimlico default.
///
/// # Errors
/// Returns an error when the override is empty or not a URL, or when the chain has no default.
pub fn bundler_urls(chain_id: u64) -> Result<Vec<Url>, BundlerListError> {
    if let Ok(raw) = std::env::var("EDW_BUNDLER_URLS") {
        return parse_urls(&raw);
    }
    match chain_id {
        1 | 11_155_111 => {
            let url = format!("{PIMLICO_RPC}/{chain_id}/rpc");
            Ok(vec![parse_url(&url)?])
        }
        _ => Err(BundlerListError::Unset { chain_id }),
    }
}

fn parse_urls(raw: &str) -> Result<Vec<Url>, BundlerListError> {
    let mut urls = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        urls.push(parse_url(part)?);
    }
    if urls.is_empty() {
        return Err(BundlerListError::Empty);
    }
    Ok(urls)
}

fn parse_url(url: &str) -> Result<Url, BundlerListError> {
    Url::parse(url).map_err(|error| BundlerListError::Url {
        url: url.to_string(),
        message: error.to_string(),
    })
}

fn already_known(error: &BundlerError) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("already known")
}

fn failed(failures: &[String]) -> BundlerError {
    if failures.is_empty() {
        return BundlerError::Other("no bundler configured".into());
    }
    BundlerError::Other(format!("every bundler failed: {}", failures.join("; ")).into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use alloy_primitives::B256;
    use kohaku_userop_kit::{
        bundler::{Bundler, BundlerError},
        signable_user_operation::SignableUserOperation,
        signed_user_operation::SignedUserOperation,
        user_operation::{UserOperationGasEstimate, UserOperationHash, UserOperationReceipt},
    };

    use super::{BundlerList, Endpoint};

    struct Script {
        label: &'static str,
        log: Arc<Mutex<Vec<String>>>,
        estimate_error: Option<&'static str>,
        send_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl Bundler for Script {
        async fn estimate_gas(
            &self,
            _op: &SignableUserOperation,
            _state_override: Option<&alloy_rpc_types_eth::state::StateOverride>,
        ) -> Result<UserOperationGasEstimate, BundlerError> {
            self.push("estimate");
            match self.estimate_error {
                Some(message) => Err(BundlerError::Other(message.into())),
                None => Ok(UserOperationGasEstimate {
                    pre_verification_gas: 1,
                    verification_gas_limit: 1,
                    call_gas_limit: 1,
                    paymaster_verification_gas_limit: None,
                    paymaster_post_op_gas_limit: None,
                    max_fee_per_gas: 1,
                    max_priority_fee_per_gas: 1,
                }),
            }
        }

        async fn send_user_operation(
            &self,
            _op: &SignedUserOperation,
        ) -> Result<UserOperationHash, BundlerError> {
            self.push("send");
            match self.send_error {
                Some(message) => Err(BundlerError::Other(message.into())),
                None => Ok(UserOperationHash(B256::ZERO)),
            }
        }

        async fn wait_for_receipt(
            &self,
            _hash: UserOperationHash,
        ) -> Result<UserOperationReceipt, BundlerError> {
            self.push("wait");
            Err(BundlerError::Timeout)
        }
    }

    impl Script {
        fn push(&self, action: &str) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} {action}", self.label));
        }
    }

    fn script(
        log: &Arc<Mutex<Vec<String>>>,
        label: &'static str,
        estimate_error: Option<&'static str>,
        send_error: Option<&'static str>,
    ) -> Endpoint {
        Endpoint {
            url: label.to_string(),
            bundler: Arc::new(Script {
                label,
                log: Arc::clone(log),
                estimate_error,
                send_error,
            }),
        }
    }

    fn list(endpoints: Vec<Endpoint>) -> BundlerList {
        BundlerList {
            endpoints,
            chain_id: 1,
            accepted: std::sync::Mutex::new(None),
        }
    }

    #[tokio::test]
    async fn the_second_bundler_is_used_when_the_first_fails() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bundlers = list(vec![
            script(&log, "a", Some("down"), None),
            script(&log, "b", None, None),
        ]);
        bundlers
            .estimate_gas(&SignableUserOperation::default(), None)
            .await
            .unwrap();
        assert_eq!(
            log.lock().unwrap().as_slice(),
            ["a estimate".to_string(), "b estimate".to_string()]
        );

        let log = Arc::new(Mutex::new(Vec::new()));
        let bundlers = list(vec![
            script(&log, "a", None, Some("refused")),
            script(&log, "b", None, None),
        ]);
        bundlers
            .send_user_operation(&SignedUserOperation::default())
            .await
            .unwrap();
        assert_eq!(bundlers.accepted_url().as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn every_bundler_failing_returns_the_error() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bundlers = list(vec![
            script(&log, "a", None, Some("down")),
            script(&log, "b", None, Some("also down")),
        ]);
        let error = bundlers
            .send_user_operation(&SignedUserOperation::default())
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("every bundler failed"), "{message}");
        assert!(message.contains("also down"), "{message}");
    }

    #[tokio::test]
    async fn a_receipt_is_polled_only_on_the_bundler_that_accepted_the_send() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bundlers = list(vec![
            script(&log, "a", None, Some("refused")),
            script(&log, "b", None, None),
        ]);
        let hash = bundlers
            .send_user_operation(&SignedUserOperation::default())
            .await
            .unwrap();
        let _ = bundlers.wait_for_receipt(hash).await;
        let entries = log.lock().unwrap().clone();
        assert!(entries.contains(&"b wait".to_string()), "{entries:?}");
        assert!(
            !entries.iter().any(|entry| entry == "a wait"),
            "{entries:?}"
        );
    }

    #[tokio::test]
    async fn already_known_is_accepted_without_trying_the_next_bundler() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bundlers = list(vec![
            script(&log, "a", None, Some("already known")),
            script(&log, "b", None, None),
        ]);
        let op = SignedUserOperation::default();
        let hash = bundlers.send_user_operation(&op).await.unwrap();
        assert_eq!(hash, op.hash(1));
        let _ = bundlers.wait_for_receipt(hash).await;
        let entries = log.lock().unwrap().clone();
        assert_eq!(entries, vec!["a send".to_string(), "a wait".to_string()]);
    }
}
