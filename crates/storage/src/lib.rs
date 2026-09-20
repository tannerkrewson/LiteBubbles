//! Persistent storage boundary. SQLite migrations are added by the storage issue.

use litebubbles_core::Conversation;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage is not initialized")]
    NotInitialized,
}

#[derive(Debug, Default)]
pub struct Store {
    conversations: Vec<Conversation>,
}

impl Store {
    pub fn open() -> Result<Self, StorageError> {
        Ok(Self::default())
    }

    pub fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }
}
