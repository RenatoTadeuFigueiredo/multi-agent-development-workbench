//! Immutable domain event model.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    ApprovalId, AttemptId, ControlId, CorrelationId, DeletionId, EventId, ExportId, InputId,
    RequestId, SessionId,
    attempt::{EffectClass, ReconciliationResolution},
    policy::ApprovalDecision,
    routing::RoutingPlan,
    value::{ContentHash, NonEmptyText, ProviderId, RoleId, Sequence},
};

/// Stable event kind used by wire and persistence adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    WorkflowTransition,
}

/// Redacted role resolution retained with a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMapping {
    pub role: RoleId,
    pub provider: ProviderId,
    pub runtime_model: String,
}

/// Sensitive or public event data. Storage adapters encrypt content-bearing variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EventPayload {
    SessionCreated {
        configuration_hash: ContentHash,
        lock_hash: ContentHash,
    },
    ConfigurationResolved {
        snapshot_hash: ContentHash,
        sources: Vec<String>,
        role_mappings: Vec<RoleMapping>,
    },
    InputRecorded {
        input_id: InputId,
        content: NonEmptyText,
    },
    RoutingPlanned {
        plan: RoutingPlan,
    },
    ClarificationRequested {
        question: NonEmptyText,
        reason: String,
    },
    ApprovalRequested {
        approval_id: ApprovalId,
        action: String,
        risk: String,
        scope: Vec<String>,
    },
    ApprovalRecorded {
        approval_id: ApprovalId,
        actor: NonEmptyText,
        decision: ApprovalDecision,
    },
    DispatchPlanned {
        attempt_id: AttemptId,
        effect_class: EffectClass,
        operation: String,
        idempotent: bool,
    },
    DispatchStarted {
        attempt_id: AttemptId,
        adapter_session_id: Option<String>,
    },
    DispatchAcknowledged {
        attempt_id: AttemptId,
        provider_request_id: Option<String>,
    },
    ProviderEvent {
        attempt_id: AttemptId,
        event_type: String,
        content: NonEmptyText,
    },
    ToolEvent {
        attempt_id: AttemptId,
        event_type: String,
        content: NonEmptyText,
    },
    PauseRequested {
        control_id: ControlId,
        actor: NonEmptyText,
    },
    SessionPaused {
        control_id: ControlId,
        actor: NonEmptyText,
    },
    SessionResumed {
        control_id: ControlId,
        actor: NonEmptyText,
    },
    SessionRedirected {
        control_id: ControlId,
        actor: NonEmptyText,
        instruction: NonEmptyText,
    },
    CancelRequested {
        control_id: ControlId,
        actor: NonEmptyText,
    },
    CancelConfirmed {
        control_id: ControlId,
    },
    OutcomeUnknown {
        attempt_id: AttemptId,
        reason: String,
        reconciliation_options: Vec<ReconciliationResolution>,
    },
    OutcomeReconciled {
        attempt_id: AttemptId,
        resolution: ReconciliationResolution,
        replacement_attempt_id: Option<AttemptId>,
    },
    SessionCompleted {
        attempt_id: Option<AttemptId>,
        summary: String,
        correlation_id: CorrelationId,
    },
    SessionFailed {
        attempt_id: Option<AttemptId>,
        summary: String,
        correlation_id: CorrelationId,
    },
    SessionCancelled {
        attempt_id: Option<AttemptId>,
        summary: String,
        correlation_id: CorrelationId,
    },
    SessionAbandoned {
        attempt_id: AttemptId,
        summary: String,
        correlation_id: CorrelationId,
    },
    SessionExported {
        export_id: ExportId,
        recipient_fingerprints: Vec<String>,
    },
    SessionDeletionRequested {
        deletion_id: DeletionId,
        actor: NonEmptyText,
    },
    SessionDeleted {
        deletion_id: DeletionId,
        key_destroyed: bool,
    },
    /// Bounded workflow phase facts (identifiers only; no prompts or payloads).
    WorkflowTransition {
        workflow_id: crate::value::WorkflowId,
        run_id: NonEmptyText,
        step_id: NonEmptyText,
        iteration: u32,
        phase: String,
        reason: String,
    },
}

impl EventPayload {
    /// Returns the stable event kind without inspecting event data.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::SessionCreated { .. } => EventKind::SessionCreated,
            Self::ConfigurationResolved { .. } => EventKind::ConfigurationResolved,
            Self::InputRecorded { .. } => EventKind::InputRecorded,
            Self::RoutingPlanned { .. } => EventKind::RoutingPlanned,
            Self::ClarificationRequested { .. } => EventKind::ClarificationRequested,
            Self::ApprovalRequested { .. } => EventKind::ApprovalRequested,
            Self::ApprovalRecorded { .. } => EventKind::ApprovalRecorded,
            Self::DispatchPlanned { .. } => EventKind::DispatchPlanned,
            Self::DispatchStarted { .. } => EventKind::DispatchStarted,
            Self::DispatchAcknowledged { .. } => EventKind::DispatchAcknowledged,
            Self::ProviderEvent { .. } => EventKind::ProviderEvent,
            Self::ToolEvent { .. } => EventKind::ToolEvent,
            Self::PauseRequested { .. } => EventKind::PauseRequested,
            Self::SessionPaused { .. } => EventKind::SessionPaused,
            Self::SessionResumed { .. } => EventKind::SessionResumed,
            Self::SessionRedirected { .. } => EventKind::SessionRedirected,
            Self::CancelRequested { .. } => EventKind::CancelRequested,
            Self::CancelConfirmed { .. } => EventKind::CancelConfirmed,
            Self::OutcomeUnknown { .. } => EventKind::OutcomeUnknown,
            Self::OutcomeReconciled { .. } => EventKind::OutcomeReconciled,
            Self::SessionCompleted { .. } => EventKind::SessionCompleted,
            Self::SessionFailed { .. } => EventKind::SessionFailed,
            Self::SessionCancelled { .. } => EventKind::SessionCancelled,
            Self::SessionAbandoned { .. } => EventKind::SessionAbandoned,
            Self::SessionExported { .. } => EventKind::SessionExported,
            Self::SessionDeletionRequested { .. } => EventKind::SessionDeletionRequested,
            Self::SessionDeleted { .. } => EventKind::SessionDeleted,
            Self::WorkflowTransition { .. } => EventKind::WorkflowTransition,
        }
    }

    /// Returns whether storage must treat the payload as sensitive.
    #[must_use]
    pub const fn is_sensitive(&self) -> bool {
        matches!(
            self,
            Self::ConfigurationResolved { .. }
                | Self::InputRecorded { .. }
                | Self::RoutingPlanned { .. }
                | Self::ClarificationRequested { .. }
                | Self::ApprovalRequested { .. }
                | Self::ProviderEvent { .. }
                | Self::ToolEvent { .. }
                | Self::SessionRedirected { .. }
                | Self::SessionCompleted { .. }
                | Self::SessionFailed { .. }
                | Self::SessionCancelled { .. }
                | Self::SessionAbandoned { .. }
        )
    }

    /// Returns the attempt ID when the event belongs to an external attempt.
    #[must_use]
    pub const fn attempt_id(&self) -> Option<AttemptId> {
        match self {
            Self::DispatchPlanned { attempt_id, .. }
            | Self::DispatchStarted { attempt_id, .. }
            | Self::DispatchAcknowledged { attempt_id, .. }
            | Self::ProviderEvent { attempt_id, .. }
            | Self::ToolEvent { attempt_id, .. }
            | Self::OutcomeUnknown { attempt_id, .. }
            | Self::OutcomeReconciled { attempt_id, .. }
            | Self::SessionAbandoned { attempt_id, .. } => Some(*attempt_id),
            Self::SessionCompleted { attempt_id, .. }
            | Self::SessionFailed { attempt_id, .. }
            | Self::SessionCancelled { attempt_id, .. } => *attempt_id,
            _ => None,
        }
    }
}

/// An event awaiting durable sequence allocation.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub session_id: SessionId,
    pub causation_request_id: Option<RequestId>,
    pub payload: EventPayload,
}

/// An immutable, durably sequenced event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedEvent {
    pub event_id: EventId,
    pub session_id: SessionId,
    pub sequence: Sequence,
    pub causation_request_id: Option<RequestId>,
    pub occurred_at: OffsetDateTime,
    pub payload: EventPayload,
}

impl PersistedEvent {
    /// Returns its stable kind.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        self.payload.kind()
    }
}

#[cfg(test)]
mod tests {
    use super::{EventKind, EventPayload};
    use crate::{InputId, value::NonEmptyText};

    #[test]
    fn input_payload_is_sensitive_and_typed() {
        let payload = EventPayload::InputRecorded {
            input_id: InputId::new(),
            content: NonEmptyText::parse("secret").expect("text"),
        };
        assert_eq!(payload.kind(), EventKind::InputRecorded);
        assert!(payload.is_sensitive());
    }
}
