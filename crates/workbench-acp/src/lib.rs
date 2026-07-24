//! Bounded ACP client transport and supervised Grok Build process lifecycle.

#![forbid(unsafe_code)]

mod adapter;
mod codec;
mod error;
mod protocol;
mod supervisor;
mod transport;

pub use adapter::GrokProviderAdapter;
pub use error::{AcpError, AcpErrorKind};
pub use protocol::{
    AcpCapabilities, AcpSession, AdapterHealth, AuthenticationStatus, CancellationOutcome,
    NormalizedUpdate, PromptControl, PromptEvent, PromptExecution, PromptOutcome, StopReason,
    UpdateKind,
};
pub use supervisor::{GrokAcpClient, GrokLaunchProfile, ShutdownReport};

/// Maximum ACP JSON frame size, excluding the newline delimiter.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// ACP protocol version supported by this adapter.
pub const ACP_PROTOCOL_VERSION: u64 = 1;
