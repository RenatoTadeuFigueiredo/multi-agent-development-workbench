use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use thiserror::Error;

/// Explicitly unavailable network capability used by every default test.
#[derive(Clone, Debug, Default)]
pub struct DenyNetwork {
    attempts: Arc<AtomicU64>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("network access is denied in the deterministic test profile")]
pub struct NetworkDenied;

impl DenyNetwork {
    /// Records and rejects an attempted network operation.
    pub fn request(&self, _redacted_target: &str) -> Result<(), NetworkDenied> {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        Err(NetworkDenied)
    }

    /// Returns how many operations attempted to use network authority.
    #[must_use]
    pub fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::Relaxed)
    }

    /// Proves that the observed test path did not request network authority.
    pub fn assert_unused(&self) -> Result<(), NetworkDenied> {
        if self.attempts() == 0 {
            Ok(())
        } else {
            Err(NetworkDenied)
        }
    }
}
