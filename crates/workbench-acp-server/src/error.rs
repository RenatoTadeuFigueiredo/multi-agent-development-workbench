/// Stable redacted ACP server failure kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpServerErrorKind {
    FrameTooLarge,
    MalformedFrame,
    InvalidRequest,
    SessionNotFound,
    Backend,
    ShuttingDown,
}

/// Redacted ACP server failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpServerError {
    kind: AcpServerErrorKind,
    message: String,
}

impl AcpServerError {
    #[must_use]
    pub fn new(kind: AcpServerErrorKind, message: impl Into<String>) -> Self {
        let mut message = message.into();
        message.truncate(256);
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> AcpServerErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
