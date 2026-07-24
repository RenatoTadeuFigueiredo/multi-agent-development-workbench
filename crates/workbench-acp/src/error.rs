use thiserror::Error;

/// Stable, redacted ACP failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpErrorKind {
    InvalidConfiguration,
    SpawnFailed,
    FrameTooLarge,
    InvalidFrame,
    ProtocolViolation,
    IncompatibleProtocol,
    CapabilityUnavailable,
    AuthenticationRequired,
    RequestFailed,
    TransportClosed,
    Timeout,
    ShuttingDown,
    ReapFailed,
}

/// Redacted ACP error that never contains provider output or operating-system detail.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct AcpError {
    kind: AcpErrorKind,
    message: &'static str,
}

impl AcpError {
    #[must_use]
    pub const fn new(kind: AcpErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> AcpErrorKind {
        self.kind
    }
}
