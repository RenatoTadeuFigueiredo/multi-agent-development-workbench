//! Stable, redacted domain failures.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::CorrelationId;

/// The closed error taxonomy exposed by protocol version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    InvalidRequest,
    UnsupportedVersion,
    FrameTooLarge,
    UnauthorizedPeer,
    SessionNotFound,
    InvalidTransition,
    CapabilityUnavailable,
    PolicyDenied,
    ApprovalRequired,
    ProviderUnavailable,
    ProviderTimeout,
    OutcomeUnknown,
    ClientLagged,
    StorageUnavailable,
    KeyStoreUnavailable,
    Internal,
}

impl FailureCategory {
    /// Whether a caller may retry when no external attempt has started.
    #[must_use]
    pub const fn conditionally_retryable(self) -> bool {
        matches!(
            self,
            Self::ProviderUnavailable
                | Self::ProviderTimeout
                | Self::StorageUnavailable
                | Self::ClientLagged
        )
    }
}

/// A user-safe failure with a stable category and fresh correlation ID.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{category:?}: {message}")]
pub struct CoreError {
    category: FailureCategory,
    message: String,
    retryable: bool,
    correlation_id: CorrelationId,
}

impl CoreError {
    /// Creates a non-retryable redacted failure.
    #[must_use]
    pub fn new(category: FailureCategory, message: impl Into<String>) -> Self {
        Self::with_retryability(category, message, false)
    }

    /// Creates a failure while enforcing the closed retryability policy.
    #[must_use]
    pub fn with_retryability(
        category: FailureCategory,
        message: impl Into<String>,
        requested_retryable: bool,
    ) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "operation failed".clone_into(&mut message);
        }
        message.truncate(512);
        Self {
            category,
            message,
            retryable: requested_retryable && category.conditionally_retryable(),
            correlation_id: CorrelationId::new(),
        }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn category(&self) -> FailureCategory {
        self.category
    }

    /// Returns the user-safe message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether a retry may be considered.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns the correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[cfg(test)]
mod tests {
    use super::{CoreError, FailureCategory};

    #[test]
    fn never_marks_policy_failures_retryable() {
        let error = CoreError::with_retryability(FailureCategory::PolicyDenied, "denied", true);
        assert!(!error.retryable());
    }

    #[test]
    fn permits_availability_retryability() {
        let error = CoreError::with_retryability(
            FailureCategory::ProviderUnavailable,
            "provider unavailable",
            true,
        );
        assert!(error.retryable());
    }
}
