use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use time::OffsetDateTime;
use workbench_core::{
    AttemptId, CoreError, EventId, FailureCategory, RequestId, SessionId,
    attempt::EffectClass,
    event::{EventKind, EventPayload, NewEvent, PersistedEvent as CorePersistedEvent},
    ports::{
        CommandCommit, CommandOutcomeStore, EventStore, RecordedCommand, TransactionalCommandStore,
    },
    value::{Cursor, Sequence},
};

use crate::{
    CreateSession, EventInput, KeyStore, SqliteStorage, StorageError,
    sqlite::{PersistedEvent, StoredCommandCommit, StoredCommandRecord},
};

/// Thread-safe adapter from core persistence ports to encrypted local storage.
pub struct CoreStorageAdapter<K> {
    storage: Arc<Mutex<SqliteStorage<K>>>,
}

impl<K> Clone for CoreStorageAdapter<K> {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
        }
    }
}

impl<K: KeyStore + 'static> CoreStorageAdapter<K> {
    #[must_use]
    pub fn new(storage: SqliteStorage<K>) -> Self {
        Self {
            storage: Arc::new(Mutex::new(storage)),
        }
    }

    /// Initializes encrypted session state without exposing `SQLite` to the daemon.
    pub async fn create_session(
        &self,
        input: CreateSession,
    ) -> Result<crate::CommandOutcome, CoreError> {
        self.run_blocking(move |storage| storage.create_session(&input))
            .await
    }

    async fn run_blocking<T, F>(&self, operation: F) -> Result<T, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut SqliteStorage<K>) -> Result<T, StorageError> + Send + 'static,
    {
        let storage = Arc::clone(&self.storage);
        tokio::task::spawn_blocking(move || {
            let mut storage = storage.lock().map_err(|_| {
                CoreError::new(
                    FailureCategory::Internal,
                    "encrypted storage synchronization failed",
                )
            })?;
            operation(&mut storage).map_err(|error| storage_error(&error))
        })
        .await
        .map_err(|_| CoreError::new(FailureCategory::Internal, "encrypted storage task failed"))?
    }
}

#[async_trait]
impl<K: KeyStore + 'static> EventStore for CoreStorageAdapter<K> {
    async fn append(
        &self,
        event: NewEvent,
        occurred_at: OffsetDateTime,
    ) -> Result<CorePersistedEvent, CoreError> {
        let event_id = EventId::new();
        let input = core_to_storage_event(event_id, event, occurred_at)?;
        self.run_blocking(move |storage| storage.append_event(&input))
            .await
            .and_then(storage_to_core_event)
    }

    async fn load_after(
        &self,
        session_id: SessionId,
        cursor: Cursor,
    ) -> Result<Vec<CorePersistedEvent>, CoreError> {
        self.run_blocking(move |storage| storage.replay(session_id.as_uuid(), cursor.get()))
            .await?
            .into_iter()
            .map(storage_to_core_event)
            .collect()
    }
}

#[async_trait]
impl<K: KeyStore + 'static> CommandOutcomeStore for CoreStorageAdapter<K> {
    async fn lookup(
        &self,
        session_id: Option<SessionId>,
        request_id: RequestId,
    ) -> Result<Option<RecordedCommand>, CoreError> {
        self.run_blocking(move |storage| {
            storage.lookup_port_command(session_id.map(SessionId::as_uuid), request_id.as_uuid())
        })
        .await
        .map(|record| {
            record.map(|record| RecordedCommand {
                method: record.method,
                canonical_parameter_hash: record.parameter_hash,
                outcome: record.outcome,
            })
        })
    }

    async fn record(
        &self,
        session_id: Option<SessionId>,
        request_id: RequestId,
        command: RecordedCommand,
    ) -> Result<(), CoreError> {
        self.run_blocking(move |storage| {
            storage.record_port_command(
                session_id.map(SessionId::as_uuid),
                request_id.as_uuid(),
                &StoredCommandRecord {
                    method: command.method,
                    parameter_hash: command.canonical_parameter_hash,
                    outcome: command.outcome,
                },
            )
        })
        .await
    }
}

#[async_trait]
impl<K: KeyStore + 'static> TransactionalCommandStore for CoreStorageAdapter<K> {
    async fn commit(
        &self,
        request_id: RequestId,
        command: RecordedCommand,
        event: NewEvent,
        occurred_at: OffsetDateTime,
    ) -> Result<CommandCommit, CoreError> {
        let input = core_to_storage_event(EventId::new(), event, occurred_at)?;
        let stored_command = StoredCommandRecord {
            method: command.method,
            parameter_hash: command.canonical_parameter_hash,
            outcome: command.outcome,
        };
        let commit = self
            .run_blocking(move |storage| {
                storage.commit_port_command(request_id.as_uuid(), &stored_command, &input)
            })
            .await?;
        match commit {
            StoredCommandCommit::Applied(event) => {
                storage_to_core_event(event).map(CommandCommit::Applied)
            }
            StoredCommandCommit::Replayed(command) => {
                Ok(CommandCommit::Replayed(RecordedCommand {
                    method: command.method,
                    canonical_parameter_hash: command.parameter_hash,
                    outcome: command.outcome,
                }))
            }
        }
    }
}

fn core_to_storage_event(
    event_id: EventId,
    event: NewEvent,
    occurred_at: OffsetDateTime,
) -> Result<EventInput, CoreError> {
    let kind = event_kind_name(event.payload.kind())?;
    let attempt_id = event.payload.attempt_id().map(AttemptId::as_uuid);
    let effect_class = match &event.payload {
        EventPayload::DispatchPlanned { effect_class, .. } => {
            Some(effect_class_name(*effect_class)?)
        }
        _ => None,
    };
    let payload = serde_json::to_value(event.payload)
        .map_err(|_| CoreError::new(FailureCategory::Internal, "event serialization failed"))?;
    Ok(EventInput {
        event_id: event_id.as_uuid(),
        session_id: event.session_id.as_uuid(),
        occurred_at,
        kind,
        causation_request_id: event.causation_request_id.map(RequestId::as_uuid),
        attempt_id,
        effect_class,
        payload,
    })
}

fn storage_to_core_event(event: PersistedEvent) -> Result<CorePersistedEvent, CoreError> {
    let payload: EventPayload = serde_json::from_value(event.payload).map_err(|_| {
        CoreError::new(
            FailureCategory::StorageUnavailable,
            "encrypted event payload is invalid",
        )
    })?;
    let expected_kind = event_kind_name(payload.kind())?;
    if event.kind != expected_kind {
        return Err(CoreError::new(
            FailureCategory::StorageUnavailable,
            "encrypted event metadata is inconsistent",
        ));
    }
    Ok(CorePersistedEvent {
        event_id: EventId::from_uuid(event.event_id),
        session_id: SessionId::from_uuid(event.session_id),
        sequence: Sequence::new(event.sequence).map_err(|_| {
            CoreError::new(
                FailureCategory::StorageUnavailable,
                "encrypted event sequence is invalid",
            )
        })?,
        causation_request_id: event.causation_request_id.map(RequestId::from_uuid),
        occurred_at: event.occurred_at,
        payload,
    })
}

fn event_kind_name(kind: EventKind) -> Result<String, CoreError> {
    serde_name(kind, "event kind serialization failed")
}

fn effect_class_name(effect_class: EffectClass) -> Result<String, CoreError> {
    serde_name(effect_class, "effect class serialization failed")
}

fn serde_name<T: serde::Serialize>(value: T, failure: &'static str) -> Result<String, CoreError> {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => Ok(name),
        _ => Err(CoreError::new(FailureCategory::Internal, failure)),
    }
}

fn storage_error(error: &StorageError) -> CoreError {
    let (category, message) = match error {
        StorageError::InvalidInput(_) | StorageError::RequestConflict => (
            FailureCategory::InvalidRequest,
            "storage request is invalid",
        ),
        StorageError::SessionNotFound => {
            (FailureCategory::SessionNotFound, "session was not found")
        }
        StorageError::KeyStoreUnavailable(_) => (
            FailureCategory::KeyStoreUnavailable,
            "platform key store is unavailable",
        ),
        StorageError::UnsafeExportPath => {
            (FailureCategory::InvalidRequest, "export path is unsafe")
        }
        StorageError::StorageUnavailable(_) | StorageError::AuthenticationFailed => (
            FailureCategory::StorageUnavailable,
            "encrypted storage is unavailable",
        ),
    };
    CoreError::new(category, message)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use time::OffsetDateTime;
    use workbench_core::{
        InputId, RequestId, SessionId,
        event::{EventPayload, NewEvent},
        ports::{
            CommandCommit, CommandOutcomeStore as _, EventStore as _, RecordedCommand,
            TransactionalCommandStore as _,
        },
        value::{Cursor, NonEmptyText},
    };

    use super::CoreStorageAdapter;
    use crate::{CreateSession, MemoryKeyStore, SqliteStorage};

    fn session_input(session_id: SessionId, request_id: RequestId) -> CreateSession {
        CreateSession {
            session_id: session_id.as_uuid(),
            request_id: request_id.as_uuid(),
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({
                "session_id": session_id.as_uuid(),
                "state": "ready"
            }),
            configuration_snapshot: json!({"version": 1}),
            lock_snapshot: json!({"version": 1}),
            initial_event_payload: json!({
                "kind": "session_created",
                "data": {
                    "configuration_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "lock_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }
            }),
        }
    }

    #[tokio::test]
    async fn typed_events_round_trip_through_core_port() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let session_id = SessionId::new();
        adapter
            .create_session(session_input(session_id, RequestId::new()))
            .await
            .expect("create session");
        let event = adapter
            .append(
                NewEvent {
                    session_id,
                    causation_request_id: Some(RequestId::new()),
                    payload: EventPayload::InputRecorded {
                        input_id: InputId::new(),
                        content: NonEmptyText::parse("secret prompt").expect("content"),
                    },
                },
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("append");
        assert_eq!(event.sequence.get(), 2);

        let replay = adapter
            .load_after(session_id, Cursor::after(1))
            .await
            .expect("replay");
        assert_eq!(replay, vec![event]);
    }

    #[tokio::test]
    async fn command_request_scope_is_session_local_and_idempotent() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let request_id = RequestId::new();
        let first_session = SessionId::new();
        let second_session = SessionId::new();
        adapter
            .create_session(session_input(first_session, RequestId::new()))
            .await
            .expect("first session");
        adapter
            .create_session(session_input(second_session, RequestId::new()))
            .await
            .expect("second session");
        let command = RecordedCommand {
            method: "session.pause".to_owned(),
            canonical_parameter_hash: "a".repeat(64),
            outcome: "{\"state\":\"paused\"}".to_owned(),
        };

        adapter
            .record(Some(first_session), request_id, command.clone())
            .await
            .expect("first session");
        adapter
            .record(Some(first_session), request_id, command.clone())
            .await
            .expect("idempotent replay");
        adapter
            .record(Some(second_session), request_id, command.clone())
            .await
            .expect("second session");
        assert_eq!(
            adapter
                .lookup(Some(first_session), request_id)
                .await
                .expect("lookup"),
            Some(command)
        );
    }

    #[tokio::test]
    async fn transactional_command_replays_without_appending_another_event() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let session_id = SessionId::new();
        let request_id = RequestId::new();
        adapter
            .create_session(session_input(session_id, RequestId::new()))
            .await
            .expect("create session");
        let event = NewEvent {
            session_id,
            causation_request_id: Some(request_id),
            payload: EventPayload::InputRecorded {
                input_id: InputId::new(),
                content: NonEmptyText::parse("atomic prompt").expect("content"),
            },
        };
        let command = RecordedCommand {
            method: "session.prompt".to_owned(),
            canonical_parameter_hash: "a".repeat(64),
            outcome: "{\"accepted\":true}".to_owned(),
        };

        let first = adapter
            .commit(
                request_id,
                command.clone(),
                event.clone(),
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("first commit");
        assert!(matches!(
            first,
            CommandCommit::Applied(ref persisted) if persisted.sequence.get() == 2
        ));
        let replay = adapter
            .commit(
                request_id,
                RecordedCommand {
                    outcome: "{\"accepted\":false}".to_owned(),
                    ..command.clone()
                },
                event,
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("idempotent replay");
        assert_eq!(replay, CommandCommit::Replayed(command));
        assert_eq!(
            adapter
                .load_after(session_id, Cursor::after(1))
                .await
                .expect("events")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn transactional_command_rejects_request_id_reuse_with_changed_parameters() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let session_id = SessionId::new();
        let request_id = RequestId::new();
        adapter
            .create_session(session_input(session_id, RequestId::new()))
            .await
            .expect("create session");
        let event = NewEvent {
            session_id,
            causation_request_id: Some(request_id),
            payload: EventPayload::InputRecorded {
                input_id: InputId::new(),
                content: NonEmptyText::parse("atomic prompt").expect("content"),
            },
        };
        let command = RecordedCommand {
            method: "session.prompt".to_owned(),
            canonical_parameter_hash: "a".repeat(64),
            outcome: "{\"accepted\":true}".to_owned(),
        };
        adapter
            .commit(
                request_id,
                command.clone(),
                event.clone(),
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("first commit");

        let error = adapter
            .commit(
                request_id,
                RecordedCommand {
                    canonical_parameter_hash: "b".repeat(64),
                    ..command
                },
                event,
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect_err("request conflict");
        assert_eq!(
            error.category(),
            workbench_core::FailureCategory::InvalidRequest
        );
        assert_eq!(
            adapter
                .load_after(session_id, Cursor::after(1))
                .await
                .expect("events")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn tombstone_retains_creation_and_deletion_command_outcomes() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let session_id = SessionId::new();
        let creation_request = RequestId::new();
        let deletion_request = RequestId::new();
        let deletion_id = uuid::Uuid::now_v7();
        adapter
            .create_session(session_input(session_id, creation_request))
            .await
            .expect("create");
        adapter
            .run_blocking(move |storage| {
                storage.append_event(&crate::EventInput {
                    event_id: uuid::Uuid::now_v7(),
                    session_id: session_id.as_uuid(),
                    occurred_at: OffsetDateTime::UNIX_EPOCH,
                    kind: "session_completed".to_owned(),
                    causation_request_id: None,
                    attempt_id: None,
                    effect_class: None,
                    payload: json!({"summary": "done"}),
                })?;
                storage.request_deletion(
                    session_id.as_uuid(),
                    deletion_id,
                    deletion_request.as_uuid(),
                    OffsetDateTime::UNIX_EPOCH,
                    "test",
                )
            })
            .await
            .expect("delete");

        let creation = adapter
            .lookup(None, creation_request)
            .await
            .expect("creation lookup")
            .expect("creation outcome");
        assert_eq!(creation.method, "session.create");
        let deletion = adapter
            .lookup(Some(session_id), deletion_request)
            .await
            .expect("deletion lookup")
            .expect("deletion outcome");
        assert_eq!(deletion.method, "session.delete");

        let replacement_session = SessionId::new();
        let replay = adapter
            .create_session(session_input(replacement_session, creation_request))
            .await
            .expect("creation replay");
        assert!(matches!(replay, crate::CommandOutcome::Replay(_)));
        assert!(
            adapter
                .load_after(replacement_session, Cursor::after(0))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_command_outcome_for_missing_session() {
        let storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let adapter = CoreStorageAdapter::new(storage);
        let error = adapter
            .record(
                Some(SessionId::new()),
                RequestId::new(),
                RecordedCommand {
                    method: "session.pause".to_owned(),
                    canonical_parameter_hash: "a".repeat(64),
                    outcome: "{}".to_owned(),
                },
            )
            .await
            .expect_err("missing session");
        assert_eq!(
            error.category(),
            workbench_core::FailureCategory::SessionNotFound
        );
        assert_eq!(error.message(), "session was not found");
    }
}
