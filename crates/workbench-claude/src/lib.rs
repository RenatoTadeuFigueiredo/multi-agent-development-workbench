//! Bounded stream-JSON adapter for a supervised Claude Code process.

#![forbid(unsafe_code)]

mod adapter;
mod codec;
mod error;
mod process;
mod protocol;

pub use adapter::ClaudeProviderAdapter;
pub use error::{ClaudeError, ClaudeErrorKind};
pub use process::{ClaudeLaunchProfile, ShutdownReport};

/// Maximum encoded JSON frame size, excluding the newline delimiter.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Locked protocol identity implemented by this adapter.
pub const CLAUDE_CODE_STREAM_PROTOCOL: &str = "claude-code-stream-json/1";
