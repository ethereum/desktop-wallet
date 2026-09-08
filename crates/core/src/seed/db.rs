use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    database::{Database, DatabaseError},
    network::NetworkId,
    seed::{Mnemonic, SeedError, SeedRecord},
};

const SEED_KEY: &[u8] = b"seed";

#[derive(Serialize, Deserialize)]
struct StoredSeed {
    phrase: String,
    network_id: NetworkId,
    profile_index: u32,
}

pub(crate) trait SeedDb: Database {
    async fn get_seed(&self) -> Result<SeedRecord, SeedError> {
        let Some(bytes) = self.get(SEED_KEY).await? else {
            return Err(SeedError::MissingSeed);
        };
        let stored: StoredSeed = postcard::from_bytes(&bytes)?;
        let phrase = Zeroizing::new(stored.phrase);
        Ok(SeedRecord::new(
            Mnemonic::parse(&phrase)?,
            stored.network_id,
            stored.profile_index,
        ))
    }

    async fn put_seed(&self, record: &SeedRecord) -> Result<(), SeedError> {
        let stored = StoredSeed {
            phrase: record.words().to_owned(),
            network_id: record.network_id,
            profile_index: record.profile_index,
        };
        let bytes = Zeroizing::new(postcard::to_stdvec(&stored)?);
        self.put(SEED_KEY, &bytes).await?;
        Ok(())
    }
}

impl<D: Database + ?Sized> SeedDb for D {}

impl From<DatabaseError> for SeedError {
    fn from(err: DatabaseError) -> Self {
        Self::Database(err)
    }
}

impl From<postcard::Error> for SeedError {
    fn from(err: postcard::Error) -> Self {
        Self::Serialization(err)
    }
}
