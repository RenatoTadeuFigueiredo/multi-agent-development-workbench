use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::PROTOCOL_V1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEvent {
    pub protocol: String,
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_request_id: Option<Uuid>,
    pub kind: EventKind,
    pub occurred_at: String,
    pub data: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionCreated,
    ConfigurationResolved,
    InputRecorded,
    RoutingPlanned,
    ClarificationRequested,
    ApprovalRequested,
    ApprovalRecorded,
    DispatchPlanned,
    DispatchStarted,
    DispatchAcknowledged,
    ProviderEvent,
    ToolEvent,
    PauseRequested,
    SessionPaused,
    SessionResumed,
    SessionRedirected,
    CancelRequested,
    CancelConfirmed,
    OutcomeUnknown,
    OutcomeReconciled,
    SessionCompleted,
    SessionFailed,
    SessionCancelled,
    SessionAbandoned,
    SessionExported,
    SessionDeletionRequested,
    SessionDeleted,
}

impl SessionEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.protocol != PROTOCOL_V1 {
            return Err("unsupported protocol major");
        }
        if self.sequence == 0 {
            return Err("event sequence must be positive");
        }
        if !self.data.is_object() {
            return Err("event data must be an object");
        }
        Ok(())
    }
}
