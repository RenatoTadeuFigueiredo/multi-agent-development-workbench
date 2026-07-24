use thiserror::Error;

/// Stable Claude adapter error categories. Process output is never included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeErrorKind {
    InvalidConfiguration,
    SpawnFailed,
    AuthenticationRequired,
    IncompatibleProtocol,
    CapabilityUnavailable,
    FrameTooLarge,
    InvalidFrame,
    ProtocolViolation,
    TransportClosed,
    Timeout,
    ShuttingDown,
    ReapFailed,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ClaudeError {
    kind: ClaudeErrorKind,
    message: &'static str,
}

impl ClaudeError {
    #[must_use]
    pub const fn new(kind: ClaudeErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> ClaudeErrorKind {
        self.kind
    }
}
