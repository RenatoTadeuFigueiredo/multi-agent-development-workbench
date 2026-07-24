use thiserror::Error;

/// Stable Codex adapter error categories. Process output is never included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexErrorKind {
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
pub struct CodexError {
    kind: CodexErrorKind,
    message: &'static str,
}

impl CodexError {
    #[must_use]
    pub const fn new(kind: CodexErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> CodexErrorKind {
        self.kind
    }
}
