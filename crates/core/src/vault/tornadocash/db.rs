use kohaku_tornadocash::provider::note::Note;

use crate::database::{Database, DatabaseError};

const NOTES_KEY: &[u8] = b"notes";

#[async_trait::async_trait]
pub trait TcVaultDb: Database {
    async fn notes(&self) -> Result<Vec<Note>, DatabaseError> {
        let serialized = self.get(NOTES_KEY).await?.unwrap_or_default();
        let notes = postcard::from_bytes(&serialized).unwrap_or_default();
        Ok(notes)
    }

    async fn append_note(&self, note: Note) -> Result<(), DatabaseError> {
        let mut notes = self.notes().await?;
        notes.push(note);

        let serialized =
            postcard::to_stdvec(&notes).map_err(|e| DatabaseError::Other(Box::new(e)))?;
        self.put(NOTES_KEY, &serialized).await
    }
}

impl<T: Database> TcVaultDb for T {}
