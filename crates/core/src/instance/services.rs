use reqwest::Url;

use super::{Instance, InstanceError};
use crate::network::{
    IndexerConfig, bundler_config::default_bundler_urls, db::NetworkDb, http::parse_http_url,
};

impl Instance {
    /// Bundler URLs in fallback order. Seeds the chain's default the first time the record is
    /// missing.
    pub async fn bundler_urls(&self) -> Result<Vec<Url>, InstanceError> {
        self.stored_bundler_urls()
            .await?
            .into_iter()
            .map(|url| Ok(parse_http_url(&url)?))
            .collect()
    }

    /// Appends a fallback URL.
    pub async fn add_bundler_url(&self, url: &str) -> Result<Vec<Url>, InstanceError> {
        let url = normalize_url(url)?;
        let mut urls = self.stored_bundler_urls().await?;
        if urls.iter().any(|existing| existing == &url) {
            return Err(InstanceError::DuplicateBundlerUrl(url));
        }
        urls.push(url);
        self.network_db().put_bundler_config(&urls).await?;
        self.bundler_urls().await
    }

    /// Drops one URL. The last remaining URL cannot be removed.
    pub async fn remove_bundler_url(&self, url: &str) -> Result<Vec<Url>, InstanceError> {
        let url = normalize_url(url)?;
        let mut urls = self.stored_bundler_urls().await?;
        let Some(index) = urls.iter().position(|existing| existing == &url) else {
            return Err(InstanceError::UnknownBundlerUrl(url));
        };
        if urls.len() == 1 {
            return Err(InstanceError::LastBundlerUrl);
        }
        urls.remove(index);
        self.network_db().put_bundler_config(&urls).await?;
        self.bundler_urls().await
    }

    /// The indexer. Seeds the chain's default the first time the record is missing.
    pub async fn indexer_config(&self) -> Result<IndexerConfig, InstanceError> {
        let records = self.network_db();
        if let Some(config) = records.get_indexer_config().await? {
            return Ok(config);
        }
        let config = IndexerConfig::default_for(self.network.id);
        records.put_indexer_config(&config).await?;
        Ok(config)
    }

    /// Replaces the indexer URL and leaves on or off as it is.
    pub async fn set_indexer_url(&self, url: &str) -> Result<IndexerConfig, InstanceError> {
        let mut config = self.indexer_config().await?;
        config.url = Some(normalize_url(url)?);
        self.network_db().put_indexer_config(&config).await?;
        Ok(config)
    }

    /// Turns the indexer on or off. On requires a URL. Off keeps the URL.
    pub async fn set_indexer_enabled(&self, enabled: bool) -> Result<IndexerConfig, InstanceError> {
        let mut config = self.indexer_config().await?;
        if enabled && config.url.is_none() {
            return Err(InstanceError::IndexerUrlRequired);
        }
        config.enabled = enabled;
        self.network_db().put_indexer_config(&config).await?;
        Ok(config)
    }

    /// The chain's default bundler list, when none is stored yet.
    pub(super) async fn install_default_bundler_config(&self) -> Result<(), InstanceError> {
        if self.network_db().get_bundler_config().await?.is_some() {
            return Ok(());
        }
        let urls = default_bundler_urls(self.network.id);
        self.network_db().put_bundler_config(&urls).await?;
        Ok(())
    }

    /// The chain's default indexer, when none is stored yet.
    pub(super) async fn install_default_indexer_config(&self) -> Result<(), InstanceError> {
        if self.network_db().get_indexer_config().await?.is_some() {
            return Ok(());
        }
        let config = IndexerConfig::default_for(self.network.id);
        self.network_db().put_indexer_config(&config).await?;
        Ok(())
    }

    async fn stored_bundler_urls(&self) -> Result<Vec<String>, InstanceError> {
        let records = self.network_db();
        if let Some(urls) = records.get_bundler_config().await? {
            return Ok(urls);
        }
        let urls = default_bundler_urls(self.network.id);
        records.put_bundler_config(&urls).await?;
        Ok(urls)
    }
}

fn normalize_url(url: &str) -> Result<String, InstanceError> {
    let trimmed = url.trim();
    parse_http_url(trimmed)?;
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::Instance;
    use crate::{instance::InstanceError, network::NetworkId};

    #[tokio::test]
    async fn a_missing_bundler_record_seeds_pimlico_on_mainnet() {
        let instance = Instance::in_memory(NetworkId(1));
        let urls = instance.bundler_urls().await.unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].as_str(), "https://public.pimlico.io/v2/1/rpc");
    }

    #[tokio::test]
    async fn the_last_bundler_url_cannot_be_removed() {
        let instance = Instance::in_memory(NetworkId(1));
        let err = instance
            .remove_bundler_url("https://public.pimlico.io/v2/1/rpc")
            .await
            .unwrap_err();
        assert!(matches!(err, InstanceError::LastBundlerUrl));
        instance
            .add_bundler_url("https://bundler.example/rpc")
            .await
            .unwrap();
        let left = instance
            .remove_bundler_url("https://public.pimlico.io/v2/1/rpc")
            .await
            .unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].as_str(), "https://bundler.example/rpc");
    }

    #[tokio::test]
    async fn a_local_instance_starts_with_no_bundler_and_the_indexer_off() {
        let instance = Instance::in_memory(NetworkId(31_337));
        instance.install_default_bundler_config().await.unwrap();
        instance.install_default_indexer_config().await.unwrap();
        assert!(instance.bundler_urls().await.unwrap().is_empty());
        let indexer = instance.indexer_config().await.unwrap();
        assert!(!indexer.enabled);
        assert!(indexer.url.is_none());
        let err = instance.set_indexer_enabled(true).await.unwrap_err();
        assert!(matches!(err, InstanceError::IndexerUrlRequired));
    }

    #[tokio::test]
    async fn turning_the_indexer_off_keeps_its_url() {
        let instance = Instance::in_memory(NetworkId(1));
        let on = instance.indexer_config().await.unwrap();
        assert_eq!(on.effective_url(), Some("https://saga.gordosoluciones.xyz"));
        let off = instance.set_indexer_enabled(false).await.unwrap();
        assert!(!off.enabled);
        assert_eq!(off.url.as_deref(), Some("https://saga.gordosoluciones.xyz"));
        assert!(off.effective_url().is_none());
        let again = instance.set_indexer_enabled(true).await.unwrap();
        assert_eq!(
            again.effective_url(),
            Some("https://saga.gordosoluciones.xyz")
        );
    }
}
