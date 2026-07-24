//! Layered Workbench configuration, snapshots, and deterministic locks.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

pub mod lock;
pub mod merge;
pub mod model;
pub mod preflight;
pub mod snapshot;
pub mod source;
pub mod validate;

pub use lock::{
    ACP_PROTOCOL, AdapterInput, CLAUDE_CODE_STREAM_PROTOCOL, CODEX_EXEC_JSONL_PROTOCOL,
    OPENROUTER_CHAT_COMPLETIONS_PROTOCOL, WorkbenchLock, canonicalize_adapter_executable,
};
pub use merge::{ConfigLayer, ResolvedConfiguration};
pub use model::WorkbenchConfiguration;
pub use preflight::{ProviderCapabilities, ResolvedModel};
pub use snapshot::ConfigurationSnapshot;
pub use validate::ConfigError;
