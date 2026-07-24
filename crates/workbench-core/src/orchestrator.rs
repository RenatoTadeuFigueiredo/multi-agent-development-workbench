//! Persist-before-effect orchestration service.

use std::sync::Arc;

use futures_util::StreamExt;

use crate::{
    AttemptId, CoreError, CorrelationId, FailureCategory, InputId, RequestId, SessionId,
    attempt::{Attempt, OperationPolicy, ReconciliationResolution},
    event::{EventPayload, NewEvent, PersistedEvent},
    ports::{Clock, EventStore, ProviderAdapter, ProviderFailure, ProviderOutput},
    routing::{PermissionScope, RoutingPlan},
    value::NonEmptyText,
};

/// Result after an explicit route reaches a durable outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Completed,
    Failed,
    OutcomeUnknown,
}

/// Complete input required to execute one explicitly selected route.
pub struct ExecutionRequest {
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub content: NonEmptyText,
    pub plan: RoutingPlan,
    pub operation: String,
    pub operation_policy: OperationPolicy,
    pub adapter: Arc<dyn ProviderAdapter>,
}

/// Coordinates domain ordering while infrastructure remains behind ports.
pub struct Orchestrator<'a> {
    events: &'a dyn EventStore,
    clock: &'a dyn Clock,
}

impl<'a> Orchestrator<'a> {
    /// Creates a domain service over durable events and an injectable clock.
    #[must_use]
    pub const fn new(events: &'a dyn EventStore, clock: &'a dyn Clock) -> Self {
        Self { events, clock }
    }

    /// Records input and route before creating one external provider attempt.
    ///
    /// # Errors
    ///
    /// Returns before provider dispatch when persistence, policy, or attempt
    /// validation fails. Failures after dispatch are durably represented as a
    /// definite failure or unknown outcome whenever storage remains available.
    #[allow(clippy::too_many_lines)]
    pub async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionOutcome, CoreError> {
        let ExecutionRequest {
            session_id,
            request_id,
            content,
            plan,
            operation,
            operation_policy,
            adapter,
        } = request;
        self.append(
            session_id,
            request_id,
            EventPayload::InputRecorded {
                input_id: InputId::new(),
                content: content.clone(),
            },
        )
        .await?;

        self.append(
            session_id,
            request_id,
            EventPayload::RoutingPlanned { plan: plan.clone() },
        )
        .await?;

        match plan.context.permission {
            PermissionScope::Denied => {
                return Err(CoreError::new(
                    FailureCategory::PolicyDenied,
                    "route is denied by effective policy",
                ));
            }
            PermissionScope::ApprovalRequired => {
                return Err(CoreError::new(
                    FailureCategory::ApprovalRequired,
                    "route requires recorded approval before dispatch",
                ));
            }
            PermissionScope::ReadOnly => {}
        }

        let mut attempt = Attempt::plan(operation, operation_policy)?;
        self.append(
            session_id,
            request_id,
            EventPayload::DispatchPlanned {
                attempt_id: attempt.id(),
                effect_class: attempt.policy().effect_class,
                operation: attempt.operation().to_owned(),
                idempotent: attempt.policy().explicitly_idempotent,
            },
        )
        .await?;

        attempt.mark_started()?;
        self.append(
            session_id,
            request_id,
            EventPayload::DispatchStarted {
                attempt_id: attempt.id(),
                adapter_session_id: None,
            },
        )
        .await?;

        let handle = match adapter.start_session().await {
            Ok(handle) => handle,
            Err(error) => {
                return self
                    .record_provider_setup_failure(session_id, request_id, attempt.id(), error)
                    .await;
            }
        };
        let prompt = crate::ports::ProviderPrompt {
            session_id,
            attempt_id: attempt.id(),
            runtime_model: plan.destination.runtime_model,
            content,
        };
        let mut stream = match adapter.prompt_stream(&handle, prompt).await {
            Ok(stream) => stream,
            Err(error) => {
                return self
                    .record_provider_setup_failure(session_id, request_id, attempt.id(), error)
                    .await;
            }
        };

        while let Some(item) = stream.next().await {
            match item {
                Ok(ProviderOutput::Acknowledged {
                    provider_request_id,
                }) => {
                    self.append(
                        session_id,
                        request_id,
                        EventPayload::DispatchAcknowledged {
                            attempt_id: attempt.id(),
                            provider_request_id,
                        },
                    )
                    .await?;
                }
                Ok(ProviderOutput::Content {
                    event_type,
                    content,
                }) => {
                    self.append(
                        session_id,
                        request_id,
                        EventPayload::ProviderEvent {
                            attempt_id: attempt.id(),
                            event_type,
                            content,
                        },
                    )
                    .await?;
                }
                Ok(ProviderOutput::Tool {
                    event_type,
                    content,
                }) => {
                    self.append(
                        session_id,
                        request_id,
                        EventPayload::ToolEvent {
                            attempt_id: attempt.id(),
                            event_type,
                            content,
                        },
                    )
                    .await?;
                }
                Ok(ProviderOutput::Completed { summary }) => {
                    self.append(
                        session_id,
                        request_id,
                        EventPayload::SessionCompleted {
                            attempt_id: Some(attempt.id()),
                            summary,
                            correlation_id: CorrelationId::new(),
                        },
                    )
                    .await?;
                    return Ok(ExecutionOutcome::Completed);
                }
                Err(failure) => {
                    return self
                        .record_stream_failure(session_id, request_id, attempt.id(), failure)
                        .await;
                }
            }
        }

        self.record_unknown(
            session_id,
            request_id,
            attempt.id(),
            "provider stream ended without a definite terminal result",
        )
        .await
    }

    async fn record_provider_setup_failure(
        &self,
        session_id: SessionId,
        request_id: RequestId,
        attempt_id: AttemptId,
        failure: ProviderFailure,
    ) -> Result<ExecutionOutcome, CoreError> {
        self.record_stream_failure(session_id, request_id, attempt_id, failure)
            .await
    }

    async fn record_stream_failure(
        &self,
        session_id: SessionId,
        request_id: RequestId,
        attempt_id: AttemptId,
        failure: ProviderFailure,
    ) -> Result<ExecutionOutcome, CoreError> {
        if failure.definite {
            self.append(
                session_id,
                request_id,
                EventPayload::SessionFailed {
                    attempt_id: Some(attempt_id),
                    summary: failure.user_safe_message,
                    correlation_id: CorrelationId::new(),
                },
            )
            .await?;
            Ok(ExecutionOutcome::Failed)
        } else {
            self.record_unknown(
                session_id,
                request_id,
                attempt_id,
                &failure.user_safe_message,
            )
            .await
        }
    }

    async fn record_unknown(
        &self,
        session_id: SessionId,
        request_id: RequestId,
        attempt_id: AttemptId,
        reason: &str,
    ) -> Result<ExecutionOutcome, CoreError> {
        self.append(
            session_id,
            request_id,
            EventPayload::OutcomeUnknown {
                attempt_id,
                reason: reason.to_owned(),
                reconciliation_options: vec![
                    ReconciliationResolution::Retry,
                    ReconciliationResolution::AcceptResult,
                    ReconciliationResolution::Abandon,
                ],
            },
        )
        .await?;
        Ok(ExecutionOutcome::OutcomeUnknown)
    }

    async fn append(
        &self,
        session_id: SessionId,
        request_id: RequestId,
        payload: EventPayload,
    ) -> Result<PersistedEvent, CoreError> {
        self.events
            .append(
                NewEvent {
                    session_id,
                    causation_request_id: Some(request_id),
                    payload,
                },
                self.clock.now(),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures_util::stream;
    use time::OffsetDateTime;

    use super::{ExecutionOutcome, ExecutionRequest, Orchestrator};
    use crate::{
        EventId, RequestId, SessionId,
        attempt::{EffectClass, OperationPolicy},
        event::{EventKind, NewEvent, PersistedEvent},
        ports::{
            AuthenticationStatus, CancellationStatus, Clock, EventStore, ProviderAdapter,
            ProviderCapabilities, ProviderFailure, ProviderOutput, ProviderSessionHandle,
            ProviderStream,
        },
        routing::{
            PermissionScope, Risk, RouteContext, RouteDestination, RoutingPlan, SelectedRule,
        },
        value::{Cursor, ModelAlias, NonEmptyText, ProviderId, RoleId, Sequence},
    };

    #[derive(Default)]
    struct MemoryStore {
        events: Arc<Mutex<Vec<PersistedEvent>>>,
        events_seen_before_effect: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl EventStore for MemoryStore {
        async fn append(
            &self,
            event: NewEvent,
            occurred_at: OffsetDateTime,
        ) -> Result<PersistedEvent, crate::CoreError> {
            let mut events = self.events.lock().expect("event lock");
            let persisted = PersistedEvent {
                event_id: EventId::new(),
                session_id: event.session_id,
                sequence: Sequence::new(u64::try_from(events.len()).expect("length") + 1)
                    .expect("sequence"),
                causation_request_id: event.causation_request_id,
                occurred_at,
                payload: event.payload,
            };
            events.push(persisted.clone());
            Ok(persisted)
        }

        async fn load_after(
            &self,
            _session_id: SessionId,
            cursor: Cursor,
        ) -> Result<Vec<PersistedEvent>, crate::CoreError> {
            Ok(self
                .events
                .lock()
                .expect("event lock")
                .iter()
                .filter(|event| event.sequence.get() > cursor.get())
                .cloned()
                .collect())
        }
    }

    struct TestClock;

    impl Clock for TestClock {
        fn now(&self) -> OffsetDateTime {
            OffsetDateTime::UNIX_EPOCH
        }
    }

    struct FakeProvider {
        events: Arc<Mutex<Vec<PersistedEvent>>>,
        events_seen_before_effect: Arc<Mutex<usize>>,
        setup_failure: Option<ProviderFailure>,
    }

    #[async_trait]
    impl ProviderAdapter for FakeProvider {
        async fn capabilities(&self) -> Result<ProviderCapabilities, crate::CoreError> {
            Ok(ProviderCapabilities {
                adapter_id: ProviderId::parse("fake").expect("provider"),
                adapter_version: "1".to_owned(),
                protocol: "fake/1".to_owned(),
                authentication: AuthenticationStatus::Available,
                capabilities: vec![],
                context_window_tokens: None,
            })
        }

        async fn authentication_status(&self) -> Result<AuthenticationStatus, crate::CoreError> {
            Ok(AuthenticationStatus::Available)
        }

        async fn start_session(
            &self,
        ) -> Result<ProviderSessionHandle, crate::ports::ProviderFailure> {
            *self.events_seen_before_effect.lock().expect("count") =
                self.events.lock().expect("events").len();
            if let Some(failure) = self.setup_failure.clone() {
                return Err(failure);
            }
            Ok(ProviderSessionHandle::new("opaque").expect("handle"))
        }

        async fn resume_session(
            &self,
            opaque_handle: &str,
        ) -> Result<ProviderSessionHandle, crate::ports::ProviderFailure> {
            Ok(ProviderSessionHandle::new(opaque_handle).expect("handle"))
        }

        async fn prompt_stream(
            &self,
            _handle: &ProviderSessionHandle,
            _prompt: crate::ports::ProviderPrompt,
        ) -> Result<ProviderStream, crate::ports::ProviderFailure> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ProviderOutput::Acknowledged {
                    provider_request_id: None,
                }),
                Ok(ProviderOutput::Completed {
                    summary: "done".to_owned(),
                }),
            ])))
        }

        async fn cancel(
            &self,
            _handle: &ProviderSessionHandle,
            _attempt_id: crate::AttemptId,
        ) -> Result<CancellationStatus, crate::CoreError> {
            Ok(CancellationStatus::Confirmed)
        }
    }

    fn plan() -> RoutingPlan {
        RoutingPlan {
            intent: "implement".to_owned(),
            destination: RouteDestination {
                role: RoleId::parse("implementer").expect("role"),
                model_alias: ModelAlias::parse("implementation").expect("alias"),
                provider: ProviderId::parse("fake").expect("provider"),
                runtime_model: "fake-model".to_owned(),
            },
            context: RouteContext {
                tools: vec![],
                data_sources: vec![],
                permission: PermissionScope::ReadOnly,
            },
            risk: Risk::Low,
            confidence: 1.0,
            selected_by: SelectedRule::Explicit,
        }
    }

    #[tokio::test]
    async fn persists_input_plan_and_attempt_before_provider_effect() {
        let store = MemoryStore::default();
        let provider = Arc::new(FakeProvider {
            events: Arc::clone(&store.events),
            events_seen_before_effect: Arc::clone(&store.events_seen_before_effect),
            setup_failure: None,
        });
        let outcome = Orchestrator::new(&store, &TestClock)
            .execute(ExecutionRequest {
                session_id: SessionId::new(),
                request_id: RequestId::new(),
                content: NonEmptyText::parse("prompt").expect("prompt"),
                plan: plan(),
                operation: "inference".to_owned(),
                operation_policy: OperationPolicy {
                    effect_class: EffectClass::PaidInference,
                    explicitly_idempotent: false,
                    material_cost: true,
                },
                adapter: provider,
            })
            .await
            .expect("execution");
        assert_eq!(outcome, ExecutionOutcome::Completed);
        let kinds: Vec<_> = store
            .events
            .lock()
            .expect("events")
            .iter()
            .map(PersistedEvent::kind)
            .collect();
        assert_eq!(
            &kinds[..4],
            &[
                EventKind::InputRecorded,
                EventKind::RoutingPlanned,
                EventKind::DispatchPlanned,
                EventKind::DispatchStarted,
            ]
        );
        assert_eq!(*store.events_seen_before_effect.lock().expect("count"), 4);
    }

    #[tokio::test]
    async fn distinguishes_definite_and_uncertain_setup_failures() {
        for (definite, expected_outcome, expected_kind) in [
            (true, ExecutionOutcome::Failed, EventKind::SessionFailed),
            (
                false,
                ExecutionOutcome::OutcomeUnknown,
                EventKind::OutcomeUnknown,
            ),
        ] {
            let store = MemoryStore::default();
            let provider = Arc::new(FakeProvider {
                events: Arc::clone(&store.events),
                events_seen_before_effect: Arc::clone(&store.events_seen_before_effect),
                setup_failure: Some(ProviderFailure {
                    category: crate::FailureCategory::ProviderUnavailable,
                    user_safe_message: "provider setup failed".to_owned(),
                    definite,
                }),
            });
            let outcome = Orchestrator::new(&store, &TestClock)
                .execute(ExecutionRequest {
                    session_id: SessionId::new(),
                    request_id: RequestId::new(),
                    content: NonEmptyText::parse("prompt").expect("prompt"),
                    plan: plan(),
                    operation: "inference".to_owned(),
                    operation_policy: OperationPolicy {
                        effect_class: EffectClass::PaidInference,
                        explicitly_idempotent: false,
                        material_cost: true,
                    },
                    adapter: provider,
                })
                .await
                .expect("durable failure");
            assert_eq!(outcome, expected_outcome);
            assert_eq!(
                store
                    .events
                    .lock()
                    .expect("events")
                    .last()
                    .map(PersistedEvent::kind),
                Some(expected_kind)
            );
        }
    }
}
