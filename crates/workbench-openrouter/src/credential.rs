use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use zeroize::Zeroizing;

use crate::{OpenRouterError, OpenRouterErrorKind};

/// Resolves opaque credential references without exposing storage details.
pub trait SecretSource: Send + Sync {
    /// Returns the secret for `credential_ref`, or `None` when absent.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the store is unavailable.
    fn resolve(&self, credential_ref: &str) -> Result<Option<Zeroizing<String>>, OpenRouterError>;
}

/// In-memory secret source used by offline tests.
#[derive(Clone, Default)]
pub struct MemorySecretSource {
    secrets: Arc<Mutex<HashMap<String, Zeroizing<String>>>>,
}

impl MemorySecretSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a secret for a credential reference.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    pub fn put(&self, credential_ref: impl Into<String>, secret: impl Into<String>) {
        self.secrets
            .lock()
            .expect("secret mutex")
            .insert(credential_ref.into(), Zeroizing::new(secret.into()));
    }

    /// Removes a secret for a credential reference.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    pub fn remove(&self, credential_ref: &str) {
        self.secrets.lock().expect("secret mutex").remove(credential_ref);
    }
}

impl SecretSource for MemorySecretSource {
    fn resolve(&self, credential_ref: &str) -> Result<Option<Zeroizing<String>>, OpenRouterError> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "credential store unavailable",
                )
            })?
            .get(credential_ref)
            .cloned())
    }
}

/// Loads a non-empty secret or returns a definite pre-dispatch failure.
///
/// # Errors
///
/// Returns when the secret is missing, empty, or the store fails.
pub fn require_secret(
    source: &dyn SecretSource,
    credential_ref: &str,
) -> Result<Zeroizing<String>, OpenRouterError> {
    match source.resolve(credential_ref)? {
        None => Err(OpenRouterError::new(
            OpenRouterErrorKind::CredentialMissing,
            "API credential is unavailable",
        )),
        Some(secret) if secret.is_empty() => Err(OpenRouterError::new(
            OpenRouterErrorKind::CredentialEmpty,
            "API credential is empty",
        )),
        Some(secret) => Ok(secret),
    }
}
