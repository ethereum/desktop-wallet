use kohaku_kv_store::backend::KvStoreBackend;

use crate::database::Database;

pub struct KvAdapter<T>(pub T);

#[async_trait::async_trait]
impl<T: Database> KvStoreBackend for KvAdapter<T> {
    async fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let x = self.0.get(key).await.ok()??;
        Some(x.to_vec())
    }

    async fn batch_put(&self, items: &[(&[u8], &[u8])]) {
        // TODO: add proper batching to the underlying Database trait
        for (key, value) in items {
            let _ = self.0.put(key, value).await;
        }
    }

    async fn delete(&self, key: &[u8]) {
        let _ = self.0.delete(key).await;
    }
}
