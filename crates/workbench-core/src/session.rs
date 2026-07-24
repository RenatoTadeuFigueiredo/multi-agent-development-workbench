//! Session state folding and control decisions.

use serde::{Deserialize, Serialize};

use crate::{
    ControlId, CoreError, FailureCategory,
    event::{EventPayload, PersistedEvent},
    value::NonEmptyText,
};

/// Folded state of one durable session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Ready,
    Running,
    Pausing,
    Paused,
    AwaitingClarification,
    AwaitingApproval,
    CancelRequested,
    OutcomeUnknown,
    Completed,
    Failed,
    Cancelled,
    Abandoned,
    Deleting,
    Deleted,
}

impl SessionState {
    /// Returns whether normal orchestration may start a new action.
    #[must_use]
    pub const fn permits_new_action(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Returns whether this state has a definite terminal outcome.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Abandoned | Self::Deleted
        )
    }
}

/// Folds one event while rejecting illegal lifecycle transitions.
///
/// # Errors
///
/// Returns `invalid_transition` when the event is illegal in the current state.
pub fn fold_event(
    current: Option<SessionState>,
    event: &PersistedEvent,
) -> Result<SessionState, CoreError> {
    transition(current, &event.payload)
}

/// Folds a complete ordered history.
///
/// # Errors
///
/// Returns an error for empty, non-contiguous, or illegal histories.
pub fn fold_history<'a>(
    events: impl IntoIterator<Item = &'a PersistedEvent>,
) -> Result<SessionState, CoreError> {
    let mut state = None;
    let mut previous_sequence: Option<crate::Sequence> = None;
    for event in events {
        if let Some(previous) = previous_sequence {
            let expected = previous.checked_next()?;
            if event.sequence != expected {
                return Err(CoreError::new(
                    FailureCategory::StorageUnavailable,
                    "session history contains a sequence gap",
                ));
            }
        }
        state = Some(fold_event(state, event)?);
        previous_sequence = Some(event.sequence);
    }
    state
        .ok_or_else(|| CoreError::new(FailureCategory::SessionNotFound, "session history is empty"))
}

#[allow(clippy::match_same_arms)]
fn transition(
    current: Option<SessionState>,
    payload: &EventPayload,
) -> Result<SessionState, CoreError> {
    use EventPayload::{
        ApprovalRecorded, ApprovalRequested, CancelConfirmed, CancelRequested,
        ClarificationRequested, DispatchStarted, OutcomeReconciled, OutcomeUnknown, PauseRequested,
        SessionAbandoned, SessionCancelled, SessionCompleted, SessionCreated, SessionDeleted,
        SessionDeletionRequested, SessionFailed, SessionPaused, SessionRedirected, SessionResumed,
    };
    use SessionState::{
        Abandoned, AwaitingApproval, AwaitingClarification, CancelRequested as Cancelling,
        Cancelled, Completed, Deleted, Deleting, Failed, OutcomeUnknown as Unknown, Paused,
        Pausing, Ready, Running,
    };

    let next = match (current, payload) {
        (None, SessionCreated { .. }) => Ready,
        (Some(Ready | Running), DispatchStarted { .. }) => Running,
        (Some(Ready), ClarificationRequested { .. }) => AwaitingClarification,
        (Some(Ready | Running), ApprovalRequested { .. }) => AwaitingApproval,
        (
            Some(AwaitingApproval),
            ApprovalRecorded {
                decision: crate::policy::ApprovalDecision::Grant,
                ..
            },
        ) => Running,
        (
            Some(AwaitingApproval),
            ApprovalRecorded {
                decision: crate::policy::ApprovalDecision::Deny,
                ..
            },
        ) => Paused,
        (Some(Running), PauseRequested { .. }) => Pausing,
        (Some(Pausing), SessionPaused { .. }) => Paused,
        (Some(Paused), SessionResumed { .. }) => Running,
        (Some(Paused), SessionRedirected { .. }) => Paused,
        (Some(AwaitingClarification), SessionRedirected { .. }) => Ready,
        (
            Some(Ready | Running | Pausing | Paused | AwaitingClarification | AwaitingApproval),
            CancelRequested { .. },
        ) => Cancelling,
        (Some(Cancelling), CancelConfirmed { .. }) => Cancelling,
        (Some(Cancelling), SessionCancelled { .. }) => Cancelled,
        (Some(Running | Cancelling), OutcomeUnknown { .. }) => Unknown,
        (
            Some(Unknown),
            OutcomeReconciled {
                resolution: crate::attempt::ReconciliationResolution::Retry,
                ..
            },
        ) => Running,
        (
            Some(Unknown),
            OutcomeReconciled {
                resolution: crate::attempt::ReconciliationResolution::AcceptResult,
                ..
            },
        ) => Completed,
        (
            Some(Unknown),
            OutcomeReconciled {
                resolution: crate::attempt::ReconciliationResolution::Abandon,
                ..
            },
        ) => Abandoned,
        (Some(Running), SessionCompleted { .. }) => Completed,
        (Some(Running), SessionFailed { .. }) => Failed,
        (Some(Unknown), SessionAbandoned { .. }) => Abandoned,
        (Some(Completed | Failed | Cancelled | Abandoned), SessionDeletionRequested { .. }) => {
            Deleting
        }
        (Some(Deleting), SessionDeleted { .. }) => Deleted,
        (Some(state), payload) if is_non_transitioning(state, payload) => state,
        _ => return Err(invalid_transition()),
    };
    Ok(next)
}

fn is_non_transitioning(state: SessionState, payload: &EventPayload) -> bool {
    matches!(
        (state, payload),
        (
            SessionState::Ready,
            EventPayload::ConfigurationResolved { .. }
                | EventPayload::InputRecorded { .. }
                | EventPayload::RoutingPlanned { .. }
                | EventPayload::DispatchPlanned { .. }
        ) | (
            SessionState::Running,
            EventPayload::DispatchPlanned { .. }
                | EventPayload::DispatchAcknowledged { .. }
                | EventPayload::ProviderEvent { .. }
                | EventPayload::ToolEvent { .. }
                | EventPayload::WorkflowTransition { .. }
                | EventPayload::RoutingPlanned { .. }
                | EventPayload::InputRecorded { .. }
        ) | (
            SessionState::Ready | SessionState::Paused,
            EventPayload::WorkflowTransition { .. }
        )
    ) || (state != SessionState::Deleted && matches!(payload, EventPayload::SessionExported { .. }))
}

fn invalid_transition() -> CoreError {
    CoreError::new(
        FailureCategory::InvalidTransition,
        "session transition is not allowed",
    )
}

/// A user session control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionControl {
    Pause {
        actor: NonEmptyText,
    },
    Resume {
        actor: NonEmptyText,
    },
    Redirect {
        actor: NonEmptyText,
        instruction: NonEmptyText,
    },
    Cancel {
        actor: NonEmptyText,
    },
}

/// Result of validating a control against folded state.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlOutcome {
    Append(EventPayload),
    AlreadyApplied,
}

/// Validates a control without mutating history.
///
/// # Errors
///
/// Returns `invalid_transition` when the control is illegal in the current state.
#[allow(clippy::match_same_arms)]
pub fn decide_control(
    state: SessionState,
    control: SessionControl,
) -> Result<ControlOutcome, CoreError> {
    let control_id = ControlId::new();
    match (state, control) {
        (SessionState::Running, SessionControl::Pause { actor }) => {
            Ok(ControlOutcome::Append(EventPayload::PauseRequested {
                control_id,
                actor,
            }))
        }
        (SessionState::Pausing | SessionState::Paused, SessionControl::Pause { .. }) => {
            Ok(ControlOutcome::AlreadyApplied)
        }
        (SessionState::Paused, SessionControl::Resume { actor }) => {
            Ok(ControlOutcome::Append(EventPayload::SessionResumed {
                control_id,
                actor,
            }))
        }
        (SessionState::Running, SessionControl::Resume { .. }) => {
            Ok(ControlOutcome::AlreadyApplied)
        }
        (
            SessionState::Paused | SessionState::AwaitingClarification,
            SessionControl::Redirect { actor, instruction },
        ) => Ok(ControlOutcome::Append(EventPayload::SessionRedirected {
            control_id,
            actor,
            instruction,
        })),
        (state, SessionControl::Cancel { actor })
            if !state.is_terminal()
                && !matches!(state, SessionState::Deleting | SessionState::OutcomeUnknown) =>
        {
            if state == SessionState::CancelRequested {
                Ok(ControlOutcome::AlreadyApplied)
            } else {
                Ok(ControlOutcome::Append(EventPayload::CancelRequested {
                    control_id,
                    actor,
                }))
            }
        }
        _ => Err(invalid_transition()),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use time::OffsetDateTime;

    use super::{SessionControl, SessionState, decide_control, fold_history};
    use crate::{
        EventId, SessionId,
        event::{EventPayload, PersistedEvent},
        value::{ContentHash, NonEmptyText, Sequence},
    };

    fn event(sequence: u64, session_id: SessionId, payload: EventPayload) -> PersistedEvent {
        PersistedEvent {
            event_id: EventId::new(),
            session_id,
            sequence: Sequence::new(sequence).expect("sequence"),
            causation_request_id: None,
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            payload,
        }
    }

    fn hash() -> ContentHash {
        ContentHash::parse("a".repeat(64)).expect("hash")
    }

    #[test]
    fn running_session_cannot_start_a_second_action() {
        assert!(SessionState::Ready.permits_new_action());
        assert!(!SessionState::Running.permits_new_action());
    }

    #[test]
    fn redirect_does_not_rewrite_prior_history() {
        let session_id = SessionId::new();
        let mut history = vec![event(
            1,
            session_id,
            EventPayload::SessionCreated {
                configuration_hash: hash(),
                lock_hash: hash(),
            },
        )];
        history.push(event(
            2,
            session_id,
            EventPayload::ClarificationRequested {
                question: NonEmptyText::parse("Which target?").expect("question"),
                reason: "route was ambiguous".to_owned(),
            },
        ));
        let prior = history.clone();
        let outcome = decide_control(
            SessionState::AwaitingClarification,
            SessionControl::Redirect {
                actor: NonEmptyText::parse("client").expect("actor"),
                instruction: NonEmptyText::parse("clarification").expect("instruction"),
            },
        )
        .expect("redirect");
        let super::ControlOutcome::Append(payload) = outcome else {
            panic!("expected event");
        };
        history.push(event(3, session_id, payload));
        assert_eq!(&history[..prior.len()], prior);
        assert_eq!(fold_history(&history).expect("fold"), SessionState::Ready);
    }

    proptest! {
        #[test]
        fn sequence_gaps_are_always_rejected(gap in 2_u64..u64::MAX) {
            let session_id = SessionId::new();
            let history = vec![
                event(1, session_id, EventPayload::SessionCreated {
                    configuration_hash: hash(),
                    lock_hash: hash(),
                }),
                event(gap, session_id, EventPayload::InputRecorded {
                    input_id: crate::InputId::new(),
                    content: NonEmptyText::parse("prompt").expect("prompt"),
                }),
            ];
            prop_assert!(fold_history(&history).is_err());
        }
    }
}
