use std::sync::{Mutex, MutexGuard};

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;
use workbench_storage::{
    CommandEventOutcome, CommandEventsOutcome, CommandOutcome, CreateSession, DeletionSummary,
    EventInput, ExportCommand, KeyStore, PersistedEvent, RecoveredAttempt, SessionMetadataPage,
    SqliteStorage, StorageError, StoredSession,
};

pub trait StorageBackend: Send + Sync {
    fn create_session(&self, input: &CreateSession) -> Result<CommandOutcome, StorageError>;
    fn append_event(&self, input: &EventInput) -> Result<PersistedEvent, StorageError>;
    fn append_events(&self, events: &[EventInput]) -> Result<Vec<PersistedEvent>, StorageError>;
    fn replay(
        &self,
        session_id: Uuid,
        after_sequence: u64,
    ) -> Result<Vec<PersistedEvent>, StorageError>;
    fn lookup_command_outcome(
        &self,
        session_id: Option<Uuid>,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
    ) -> Result<Option<Value>, StorageError>;
    fn record_command_outcome(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
    ) -> Result<CommandOutcome, StorageError>;
    fn commit_command_event(
        &self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        event: &EventInput,
    ) -> Result<CommandEventOutcome, StorageError>;
    fn commit_command_events(
        &self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        events: &[EventInput],
    ) -> Result<CommandEventsOutcome, StorageError>;
    fn load_session_state(&self, session_id: Uuid) -> Result<String, StorageError>;
    fn load_session(&self, session_id: Uuid) -> Result<StoredSession, StorageError>;
    fn load_sessions(&self) -> Result<Vec<StoredSession>, StorageError>;
    fn list_session_metadata(
        &self,
        limit: u16,
        before_session_id: Option<Uuid>,
    ) -> Result<SessionMetadataPage, StorageError>;
    fn is_deleted(&self, session_id: Uuid) -> Result<bool, StorageError>;
    fn recover_uncertain_attempts(
        &self,
        occurred_at: OffsetDateTime,
    ) -> Result<Vec<RecoveredAttempt>, StorageError>;
    fn resume_session_creations(&self) -> Result<usize, StorageError>;
    fn resume_deletions(&self) -> Result<Vec<DeletionSummary>, StorageError>;
    fn resume_exports(&self) -> Result<Vec<PersistedEvent>, StorageError>;
    fn ensure_session_deletable(&self, session_id: Uuid) -> Result<(), StorageError>;
    fn request_deletion(
        &self,
        session_id: Uuid,
        deletion_id: Uuid,
        request_id: Uuid,
        occurred_at: OffsetDateTime,
        actor: &str,
    ) -> Result<DeletionSummary, StorageError>;
    fn execute_export(&self, command: &ExportCommand) -> Result<CommandEventOutcome, StorageError>;
}

pub struct LockedStorage<K> {
    inner: Mutex<SqliteStorage<K>>,
}

impl<K> LockedStorage<K> {
    pub fn new(storage: SqliteStorage<K>) -> Self {
        Self {
            inner: Mutex::new(storage),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, SqliteStorage<K>>, StorageError> {
        self.inner
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))
    }
}

impl<K: KeyStore + 'static> StorageBackend for LockedStorage<K> {
    fn create_session(&self, input: &CreateSession) -> Result<CommandOutcome, StorageError> {
        self.lock()?.create_session(input)
    }

    fn append_event(&self, input: &EventInput) -> Result<PersistedEvent, StorageError> {
        self.lock()?.append_event(input)
    }

    fn append_events(&self, events: &[EventInput]) -> Result<Vec<PersistedEvent>, StorageError> {
        self.lock()?.append_events(events)
    }

    fn replay(
        &self,
        session_id: Uuid,
        after_sequence: u64,
    ) -> Result<Vec<PersistedEvent>, StorageError> {
        self.lock()?.replay(session_id, after_sequence)
    }

    fn lookup_command_outcome(
        &self,
        session_id: Option<Uuid>,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
    ) -> Result<Option<Value>, StorageError> {
        self.lock()?
            .lookup_command_outcome(session_id, request_id, method, parameters)
    }

    fn record_command_outcome(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
    ) -> Result<CommandOutcome, StorageError> {
        self.lock()?.record_command_outcome(
            &format!("session/{session_id}"),
            request_id,
            Some(session_id),
            method,
            parameters,
            outcome,
        )
    }

    fn commit_command_event(
        &self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        event: &EventInput,
    ) -> Result<CommandEventOutcome, StorageError> {
        self.lock()?
            .commit_command_event(request_id, method, parameters, outcome, event)
    }

    fn commit_command_events(
        &self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        events: &[EventInput],
    ) -> Result<CommandEventsOutcome, StorageError> {
        self.lock()?
            .commit_command_events(request_id, method, parameters, outcome, events)
    }

    fn load_session_state(&self, session_id: Uuid) -> Result<String, StorageError> {
        self.lock()?.load_session_state(session_id)
    }

    fn load_session(&self, session_id: Uuid) -> Result<StoredSession, StorageError> {
        self.lock()?.load_session(session_id)
    }

    fn load_sessions(&self) -> Result<Vec<StoredSession>, StorageError> {
        self.lock()?.load_sessions()
    }

    fn list_session_metadata(
        &self,
        limit: u16,
        before_session_id: Option<Uuid>,
    ) -> Result<SessionMetadataPage, StorageError> {
        self.lock()?.list_session_metadata(limit, before_session_id)
    }

    fn is_deleted(&self, session_id: Uuid) -> Result<bool, StorageError> {
        self.lock()?.is_deleted(session_id)
    }

    fn recover_uncertain_attempts(
        &self,
        occurred_at: OffsetDateTime,
    ) -> Result<Vec<RecoveredAttempt>, StorageError> {
        self.lock()?.recover_uncertain_attempts(occurred_at)
    }

    fn resume_deletions(&self) -> Result<Vec<DeletionSummary>, StorageError> {
        self.lock()?.resume_deletions()
    }

    fn resume_session_creations(&self) -> Result<usize, StorageError> {
        self.lock()?.resume_session_creations()
    }

    fn resume_exports(&self) -> Result<Vec<PersistedEvent>, StorageError> {
        self.lock()?.resume_exports()
    }

    fn ensure_session_deletable(&self, session_id: Uuid) -> Result<(), StorageError> {
        self.lock()?.ensure_session_deletable(session_id)
    }

    fn request_deletion(
        &self,
        session_id: Uuid,
        deletion_id: Uuid,
        request_id: Uuid,
        occurred_at: OffsetDateTime,
        actor: &str,
    ) -> Result<DeletionSummary, StorageError> {
        self.lock()?
            .request_deletion(session_id, deletion_id, request_id, occurred_at, actor)
    }

    fn execute_export(&self, command: &ExportCommand) -> Result<CommandEventOutcome, StorageError> {
        self.lock()?.execute_export(command)
    }
}
