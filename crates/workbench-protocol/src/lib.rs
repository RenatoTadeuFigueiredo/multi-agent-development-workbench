//! Versioned local client protocol.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

pub mod codec;
pub mod command;
pub mod event;
pub mod response;
pub mod subscription;
pub mod validation;

pub use codec::{MAX_FRAME_BYTES, NdjsonCodec, ProtocolCodecError};
pub use command::{ClientCommand, Command};
pub use event::{EventKind, SessionEvent};
pub use response::{ErrorCode, ProtocolError, ServerReply};
pub use subscription::{SubscriptionError, SubscriptionQueue, replay_after};

pub const PROTOCOL_V1: &str = "workbench/1";
