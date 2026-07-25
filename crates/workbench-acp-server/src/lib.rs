//! ACP v1 agent stdio bridge onto the Workbench daemon protocol.

#![forbid(unsafe_code)]

mod bridge;
mod error;
mod frame;

pub use bridge::{AcpAgentServer, BridgeBackend, DaemonSocketBackend, InProcessBackend};
pub use error::{AcpServerError, AcpServerErrorKind};
pub use frame::{MAX_FRAME_BYTES, decode_line, encode_message};

/// Agent name advertised during ACP initialize.
pub const AGENT_NAME: &str = "workbench";

/// ACP protocol major version supported by this bridge.
pub const ACP_PROTOCOL_VERSION: u64 = 1;
