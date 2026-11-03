use std::{sync::Arc, time::Duration};

use railgun_userop_kit::{
    bundler::{Bundler, BundlerError, GasPrice, pimlico::PimlicoBundler},
    signed_user_operation::SignedUserOperation,
    user_operation::{UserOperationGasEstimate, UserOperationHash, UserOperationReceipt},
};
pub use reqwest::Url;

use crate::network::http::outbound_http;

const BUNDLER_TIMEOUT: Duration = Duration::from_mins(3);

/// Bundlers tried in order. A receipt stays on the one that accepted the send.
pub struct RailgunBundlers {
    endpoints: Vec<Endpoint>,
    chain_id: u64,
    accepted: std::sync::Mutex<Option<usize>>,
}

struct Endpoint {
    url: String,
    bundler: Arc<dyn Bundler + Send + Sync>,
}

#[derive(Debug, thiserror::Error)]
pub enum RailgunBundlersError {
    #[error("no bundler URL")]
    Empty,
    #[error("invalid bundler url `{url}`: {message}")]
    Url { url: String, message: String },
    #[error("bundler http client: {0}")]
    Client(String),
}

impl RailgunBundlers {
    /// One Pimlico client per URL, sharing the outbound HTTP client.
    ///
    /// # Errors
    /// Returns an error when `urls` is empty or the HTTP client cannot be built.
    pub fn from_urls(chain_id: u64, urls: &[Url]) -> Result<Self, RailgunBundlersError> {
        if urls.is_empty() {
            return Err(RailgunBundlersError::Empty);
        }
        let client = outbound_http()
            .timeout(BUNDLER_TIMEOUT)
            .build()
            .map_err(|error| RailgunBundlersError::Client(error.to_string()))?;
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
impl Bundler for RailgunBundlers {
    async fn gas_price(&self) -> Result<Option<GasPrice>, BundlerError> {
        let mut failures = Vec::new();
        for endpoint in &self.endpoints {
            match endpoint.bundler.gas_price().await {
                Ok(price) => return Ok(price),
                Err(error) => failures.push(format!("{}: {error}", endpoint.url)),
            }
        }
        Err(failed(&failures))
    }

    async fn estimate_gas(
        &self,
        op: &railgun_userop_kit::signable_user_operation::SignableUserOperation,
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

    async fn receipt(
        &self,
        hash: UserOperationHash,
    ) -> Result<Option<UserOperationReceipt>, BundlerError> {
        let Some(index) = self.accepted_index()? else {
            let mut failures = Vec::new();
            let mut reached = false;
            for (index, endpoint) in self.endpoints.iter().enumerate() {
                match endpoint.bundler.receipt(hash).await {
                    Ok(Some(receipt)) => {
                        self.record(index)?;
                        return Ok(Some(receipt));
                    }
                    Ok(None) => reached = true,
                    Err(error) => failures.push(format!("{}: {error}", endpoint.url)),
                }
            }
            return if reached {
                Ok(None)
            } else {
                Err(failed(&failures))
            };
        };
        let Some(endpoint) = self.endpoints.get(index) else {
            return Err(BundlerError::Other(
                "accepted bundler is no longer in the list".into(),
            ));
        };
        endpoint.bundler.receipt(hash).await
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
