//! Encrypted `SQLite` persistence and platform key-store adapters.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod crypto;
mod export;
mod key_store;
mod ports;
mod sqlite;

pub use crypto::{AssociatedData, EncryptedPayload, SecretKey};
pub use export::{ExportCommand, ExportSummary, recipient_fingerprints};
pub use key_store::{KeyManager, KeyStore, MemoryKeyStore, PlatformKeyStore};
pub use ports::CoreStorageAdapter;
pub use sqlite::{
    CommandEventOutcome, CommandEventsOutcome, CommandOutcome, CreateSession, DeletionSummary,
    EventInput, PersistedEvent, RecoveredAttempt, SqliteStorage, StoredSession,
};

use thiserror::Error;

/// Fail-closed errors returned by the encrypted persistence boundary.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("invalid storage input: {0}")]
    InvalidInput(&'static str),
    #[error("encrypted storage is unavailable")]
    StorageUnavailable(#[source] Option<Box<dyn std::error::Error + Send + Sync>>),
    #[error("platform key store is unavailable")]
    KeyStoreUnavailable(#[source] Option<Box<dyn std::error::Error + Send + Sync>>),
    #[error("encrypted payload authentication failed")]
    AuthenticationFailed,
    #[error("session was not found")]
    SessionNotFound,
    #[error("request identifier conflicts with a recorded command")]
    RequestConflict,
    #[error("requested export already exists or is unsafe")]
    UnsafeExportPath,
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::StorageUnavailable(Some(Box::new(error)))
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::StorageUnavailable(Some(Box::new(error)))
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::StorageUnavailable(Some(Box::new(error)))
    }
}
