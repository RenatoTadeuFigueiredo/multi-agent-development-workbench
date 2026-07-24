//! Platform keyring secret source for production credential resolution.

use zeroize::Zeroizing;

use crate::{OpenRouterError, OpenRouterErrorKind, credential::SecretSource};

const PLATFORM_SERVICE: &str = "multi-agent-development-workbench";

/// Resolves `platform:` / `keychain:` / `secret-service:` handles via keyring.
#[derive(Debug, Clone, Default)]
pub struct PlatformSecretSource;

impl PlatformSecretSource {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn account_id(credential_ref: &str) -> Option<&str> {
        credential_ref
            .strip_prefix("platform:")
            .or_else(|| credential_ref.strip_prefix("keychain:"))
            .or_else(|| credential_ref.strip_prefix("secret-service:"))
    }
}

impl SecretSource for PlatformSecretSource {
    fn resolve(&self, credential_ref: &str) -> Result<Option<Zeroizing<String>>, OpenRouterError> {
        let Some(account) = Self::account_id(credential_ref) else {
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::InvalidConfig,
                "credential_ref scheme is unsupported",
            ));
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let entry = keyring::Entry::new(PLATFORM_SERVICE, account).map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "credential store unavailable",
                )
            })?;
            match entry.get_password() {
                Ok(secret) => Ok(Some(Zeroizing::new(secret))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(OpenRouterError::new(
                    OpenRouterErrorKind::Unavailable,
                    "credential store unavailable",
                )),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = account;
            Err(OpenRouterError::new(
                OpenRouterErrorKind::Unavailable,
                "credential store unavailable on this platform",
            ))
        }
    }
}
