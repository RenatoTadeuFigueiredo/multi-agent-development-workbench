use workbench_core::{CoreError, FailureCategory, ports::ProviderFailure};

/// Stable redacted OpenRouter failure kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRouterErrorKind {
    InvalidConfig,
    CredentialMissing,
    CredentialEmpty,
    BudgetExceeded,
    Transport,
    ResponseTooLarge,
    MalformedStream,
    Unavailable,
    Cancelled,
    OutcomeUnknown,
    ShuttingDown,
}

/// Redacted OpenRouter adapter failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRouterError {
    kind: OpenRouterErrorKind,
    message: String,
}

impl OpenRouterError {
    #[must_use]
    pub fn new(kind: OpenRouterErrorKind, message: impl Into<String>) -> Self {
        let mut message = message.into();
        message.truncate(256);
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> OpenRouterErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub fn into_core(self) -> CoreError {
        let category = match self.kind {
            OpenRouterErrorKind::InvalidConfig => FailureCategory::InvalidRequest,
            OpenRouterErrorKind::CredentialMissing | OpenRouterErrorKind::CredentialEmpty => {
                FailureCategory::ProviderUnavailable
            }
            OpenRouterErrorKind::BudgetExceeded => FailureCategory::PolicyDenied,
            OpenRouterErrorKind::Transport
            | OpenRouterErrorKind::ResponseTooLarge
            | OpenRouterErrorKind::MalformedStream
            | OpenRouterErrorKind::Unavailable
            | OpenRouterErrorKind::ShuttingDown => FailureCategory::ProviderUnavailable,
            OpenRouterErrorKind::Cancelled => FailureCategory::InvalidTransition,
            OpenRouterErrorKind::OutcomeUnknown => FailureCategory::OutcomeUnknown,
        };
        CoreError::new(category, self.message)
    }

    #[must_use]
    pub fn into_provider_failure(self, definite: bool) -> ProviderFailure {
        let category = match self.kind {
            OpenRouterErrorKind::BudgetExceeded => FailureCategory::PolicyDenied,
            OpenRouterErrorKind::CredentialMissing | OpenRouterErrorKind::CredentialEmpty => {
                FailureCategory::ProviderUnavailable
            }
            OpenRouterErrorKind::OutcomeUnknown => FailureCategory::OutcomeUnknown,
            OpenRouterErrorKind::InvalidConfig => FailureCategory::InvalidRequest,
            _ => FailureCategory::ProviderUnavailable,
        };
        ProviderFailure {
            category,
            user_safe_message: self.message,
            definite,
        }
    }
}

impl From<OpenRouterError> for CoreError {
    fn from(value: OpenRouterError) -> Self {
        value.into_core()
    }
}
