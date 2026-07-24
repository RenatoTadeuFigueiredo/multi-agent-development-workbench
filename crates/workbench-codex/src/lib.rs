//! Bounded `codex exec --json` adapter for a supervised Codex CLI process.

#![forbid(unsafe_code)]

mod adapter;
mod codec;
mod error;
mod process;
mod protocol;

pub use adapter::CodexProviderAdapter;
pub use error::{CodexError, CodexErrorKind};
pub use process::{CodexLaunchProfile, ShutdownReport};

/// Maximum encoded JSON frame size, excluding the newline delimiter.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Locked protocol identity implemented by this adapter.
pub const CODEX_EXEC_JSONL_PROTOCOL: &str = "codex-exec-jsonl/1";

/// Test-only frame decode surface used by acceptance harnesses.
#[doc(hidden)]
pub fn codec_test_decode(bytes: &[u8]) -> Result<(), CodexErrorKind> {
    codec::decode_frame(bytes)
        .map(|_| ())
        .map_err(|error| error.kind())
}
