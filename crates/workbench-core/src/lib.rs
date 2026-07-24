//! Provider-independent orchestration domain.

#![forbid(unsafe_code)]

pub mod attempt;
pub mod error;
pub mod event;
pub mod identity;
pub mod orchestrator;
pub mod policy;
pub mod ports;
pub mod routing;
pub mod session;
pub mod value;

pub use error::{CoreError, FailureCategory};
pub use identity::{
    ApprovalId, AttemptId, ControlId, CorrelationId, DeletionId, EventId, ExportId, InputId,
    RequestId, SessionId,
};
pub use value::{ContentHash, Cursor, Sequence};
