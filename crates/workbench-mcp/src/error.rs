//! Stable, redacted MCP gateway error categories.

use thiserror::Error;
use workbench_core::FailureCategory;

/// Public error categories for MCP lifecycle and tool dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpErrorKind {
    InvalidConfiguration,
    PinMismatch,
    Unavailable,
    PolicyDenied,
    ApprovalRequired,
    ApprovalDenied,
    TransportFailed,
    ResponseTooLarge,
    RedirectRejected,
    Timeout,
    Cancelled,
    OutcomeUnknown,
    ShuttingDown,
    ReapFailed,
    Internal,
}

impl McpErrorKind {
    /// Maps to the closed domain failure taxonomy.
    #[must_use]
    pub const fn failure_category(self) -> FailureCategory {
        match self {
            Self::InvalidConfiguration => FailureCategory::InvalidRequest,
            Self::PinMismatch | Self::Unavailable | Self::TransportFailed => {
                FailureCategory::ProviderUnavailable
            }
            Self::PolicyDenied | Self::ApprovalDenied => FailureCategory::PolicyDenied,
            Self::ApprovalRequired => FailureCategory::ApprovalRequired,
            Self::ResponseTooLarge | Self::RedirectRejected => FailureCategory::InvalidRequest,
            Self::Timeout => FailureCategory::ProviderTimeout,
            Self::Cancelled | Self::OutcomeUnknown => FailureCategory::OutcomeUnknown,
            Self::ShuttingDown => FailureCategory::InvalidTransition,
            Self::ReapFailed | Self::Internal => FailureCategory::Internal,
        }
    }
}

/// Redacted gateway failure. Never carries secrets, paths, or raw payloads.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct McpError {
    kind: McpErrorKind,
    message: &'static str,
}

impl McpError {
    #[must_use]
    pub const fn new(kind: McpErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> McpErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    #[must_use]
    pub const fn failure_category(&self) -> FailureCategory {
        self.kind.failure_category()
    }
}

pub(crate) const fn pin_mismatch() -> McpError {
    McpError::new(McpErrorKind::PinMismatch, "MCP artifact pin mismatch")
}

pub(crate) const fn unavailable() -> McpError {
    McpError::new(McpErrorKind::Unavailable, "MCP server is unavailable")
}

pub(crate) const fn policy_denied() -> McpError {
    McpError::new(McpErrorKind::PolicyDenied, "tool denied by policy")
}

pub(crate) const fn approval_required() -> McpError {
    McpError::new(
        McpErrorKind::ApprovalRequired,
        "tool requires human approval",
    )
}

pub(crate) const fn approval_denied() -> McpError {
    McpError::new(McpErrorKind::ApprovalDenied, "tool approval was denied")
}

pub(crate) const fn response_too_large() -> McpError {
    McpError::new(
        McpErrorKind::ResponseTooLarge,
        "MCP response exceeded size ceiling",
    )
}

pub(crate) const fn redirect_rejected() -> McpError {
    McpError::new(
        McpErrorKind::RedirectRejected,
        "MCP redirect to unpinned host rejected",
    )
}

pub(crate) const fn transport_failed() -> McpError {
    McpError::new(McpErrorKind::TransportFailed, "MCP transport failed")
}

pub(crate) const fn shutting_down() -> McpError {
    McpError::new(McpErrorKind::ShuttingDown, "MCP gateway is shutting down")
}

pub(crate) const fn reap_failed() -> McpError {
    McpError::new(McpErrorKind::ReapFailed, "MCP child could not be reaped")
}

pub(crate) const fn outcome_unknown() -> McpError {
    McpError::new(
        McpErrorKind::OutcomeUnknown,
        "MCP call outcome is unknown after start",
    )
}

pub(crate) const fn invalid_configuration() -> McpError {
    McpError::new(
        McpErrorKind::InvalidConfiguration,
        "MCP configuration is invalid",
    )
}
