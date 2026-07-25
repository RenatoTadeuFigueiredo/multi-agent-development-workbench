//! Durable `OpenRouter` spend store backed by encrypted `SQLite` sessions.

use std::path::PathBuf;

use uuid::Uuid;
use workbench_openrouter::{DurableSpendStore, OpenRouterError, OpenRouterErrorKind};
use workbench_storage::{PlatformKeyStore, SqliteStorage};

/// Path-based durable spend store that opens the workstation database briefly.
#[derive(Debug, Clone)]
pub struct PathSpendStore {
    database: PathBuf,
}

impl PathSpendStore {
    #[must_use]
    pub fn new(database: impl Into<PathBuf>) -> Self {
        Self {
            database: database.into(),
        }
    }
}

impl DurableSpendStore for PathSpendStore {
    fn load_spends(&self) -> Result<Vec<(Uuid, u64)>, OpenRouterError> {
        let storage =
            SqliteStorage::open(&self.database, PlatformKeyStore::new()).map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "cost ledger storage unavailable",
                )
            })?;
        storage.load_all_session_spends().map_err(|_| {
            OpenRouterError::new(
                OpenRouterErrorKind::Unavailable,
                "cost ledger storage unavailable",
            )
        })
    }

    fn persist_spend(
        &self,
        session_id: Uuid,
        spend_usd_micros: u64,
    ) -> Result<(), OpenRouterError> {
        let storage =
            SqliteStorage::open(&self.database, PlatformKeyStore::new()).map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "cost ledger storage unavailable",
                )
            })?;
        storage
            .store_session_spend_usd_micros(session_id, spend_usd_micros)
            .map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "cost ledger persist failed",
                )
            })
    }
}
