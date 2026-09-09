use kohaku_tornadocash::provider::note::Note;

use crate::database::Database;

#[async_trait::async_trait]
pub trait TcVaultDb: Database {
    async fn notes(&self) -> Vec<Note> {
        todo!()
    }

    async fn append_note(&self, note: &Note) {
        todo!()
    }
}

impl<T: Database> TcVaultDb for T {}
