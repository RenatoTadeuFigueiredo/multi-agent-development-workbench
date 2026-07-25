use std::{fs, path::Path};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde_json::{Map, Value};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{AssociatedData, EncryptedPayload, KeyManager, KeyStore, SecretKey, StorageError};

const MIGRATION_001: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_002: &str = include_str!("../migrations/0002_session_spend.sql");
const TERMINAL_KINDS: &[&str] = &[
    "session_completed",
    "session_failed",
    "session_cancelled",
    "session_abandoned",
];

#[derive(Debug, Clone)]
pub struct EventInput {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub kind: String,
    pub causation_request_id: Option<Uuid>,
    pub attempt_id: Option<Uuid>,
    pub effect_class: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedEvent {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub sequence: u64,
    pub occurred_at: OffsetDateTime,
    pub kind: String,
    pub causation_request_id: Option<Uuid>,
    pub attempt_id: Option<Uuid>,
    pub effect_class: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct CreateSession {
    pub session_id: Uuid,
    pub request_id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub request_parameters: Value,
    pub command_outcome: Value,
    pub configuration_snapshot: Value,
    pub lock_snapshot: Value,
    pub initial_event_payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSession {
    pub session_id: Uuid,
    pub state: String,
    pub terminal_at: Option<OffsetDateTime>,
    pub configuration_snapshot: Value,
    pub lock_snapshot: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSessionMetadata {
    pub session_id: Uuid,
    pub state: String,
    pub created_at: OffsetDateTime,
    pub terminal_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataPage {
    pub sessions: Vec<StoredSessionMetadata>,
    pub next_before_session_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    Recorded(Value),
    Replay(Value),
}

/// Result of atomically committing a command outcome with its primary event.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandEventOutcome {
    Recorded {
        event: PersistedEvent,
        outcome: Value,
    },
    Replay(Value),
}

/// Result of atomically committing a command outcome with all of its local facts.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandEventsOutcome {
    Recorded {
        events: Vec<PersistedEvent>,
        outcome: Value,
    },
    Replay(Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct RecoveredAttempt {
    pub session_id: Uuid,
    pub attempt_id: Uuid,
    pub event_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionSummary {
    pub session_id: Uuid,
    pub deletion_id: Uuid,
    pub key_destroyed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredCommandRecord {
    pub method: String,
    pub parameter_hash: String,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StoredCommandCommit {
    Applied(PersistedEvent),
    Replayed(StoredCommandRecord),
}

#[derive(Debug)]
struct SessionCreationJournal {
    session_id: Uuid,
    request_id: Uuid,
    parameter_hash: String,
    key_id: String,
    state: String,
}

/// `SQLite` adapter that keeps all sensitive values encrypted at the application boundary.
pub struct SqliteStorage<K> {
    pub(crate) connection: Connection,
    pub(crate) keys: KeyManager<K>,
}

impl<K: KeyStore> SqliteStorage<K> {
    pub fn open(path: &Path, key_store: K) -> Result<Self, StorageError> {
        let path = validated_database_path(path)?;
        let existed = path.exists();
        if existed {
            validate_private_storage_file(&path)?;
            validate_existing_sidecars(&path)?;
        }
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        if !existed {
            set_private_permissions(&path)?;
        }
        let location_scope = database_location_scope(&path);
        let storage = Self::initialize(connection, key_store, true, &location_scope)?;
        secure_sidecars(&path)?;
        Ok(storage)
    }

    pub fn open_in_memory(key_store: K) -> Result<Self, StorageError> {
        Self::initialize(
            Connection::open_in_memory()?,
            key_store,
            false,
            b"in-memory",
        )
    }

    fn initialize(
        connection: Connection,
        key_store: K,
        enable_wal: bool,
        location_scope: &[u8],
    ) -> Result<Self, StorageError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "trusted_schema", "OFF")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "secure_delete", "FAST")?;
        if enable_wal {
            let mode: String =
                connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                return Err(StorageError::StorageUnavailable(None));
            }
        }

        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(MIGRATION_001)?;
                connection.execute_batch(MIGRATION_002)?;
            }
            1 => connection.execute_batch(MIGRATION_002)?,
            2 => {}
            _ => {
                return Err(StorageError::InvalidInput(
                    "unsupported storage schema version",
                ));
            }
        }
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(StorageError::StorageUnavailable(None));
        }
        connection.execute(
            "INSERT OR IGNORE INTO storage_identity (singleton, storage_id) VALUES (1, ?1)",
            [Uuid::now_v7().to_string()],
        )?;
        let storage_id = parse_uuid(&connection.query_row(
            "SELECT storage_id FROM storage_identity WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )?)?;

        Ok(Self {
            connection,
            keys: KeyManager::for_storage(key_store, storage_id, location_scope),
        })
    }

    pub fn create_session(
        &mut self,
        input: &CreateSession,
    ) -> Result<CommandOutcome, StorageError> {
        let parameter_hash = canonical_hash(&input.request_parameters)?;
        if let Some(record) = self.lookup_port_command(None, input.request_id)? {
            if record.method == "session.create" && record.parameter_hash == parameter_hash {
                return Ok(CommandOutcome::Replay(serde_json::from_str(
                    &record.outcome,
                )?));
            }
            return Err(StorageError::RequestConflict);
        }

        self.reconcile_create_retry(input.session_id, input.request_id, &parameter_hash)?;
        let key_id = self.keys.session_key_id(input.session_id);
        self.connection.execute(
            "INSERT INTO session_creation_journal (
                session_id, request_id, parameter_hash, key_id, state
             ) VALUES (?1, ?2, ?3, ?4, 'prepared')",
            params![
                input.session_id.to_string(),
                input.request_id.to_string(),
                parameter_hash,
                key_id
            ],
        )?;

        let result = self.finish_create_session(input, &parameter_hash, &key_id);
        if result.is_err() {
            let _ignored = self.reconcile_creation_by_request(input.request_id);
        }
        result
    }

    fn finish_create_session(
        &mut self,
        input: &CreateSession,
        parameter_hash: &str,
        journal_key_id: &str,
    ) -> Result<CommandOutcome, StorageError> {
        let key_id = self.keys.create_session_key(input.session_id)?;
        if key_id != journal_key_id {
            return Err(StorageError::StorageUnavailable(None));
        }
        self.mark_creation_key_created(input, parameter_hash, &key_id)?;

        let key = self.keys.session_key(input.session_id)?;
        let configuration = encrypt_json(
            &key,
            input.session_id,
            "configuration",
            0,
            "configuration_snapshot",
            &input.configuration_snapshot,
        )?;
        let lock = encrypt_json(
            &key,
            input.session_id,
            "lock",
            0,
            "session_lock",
            &input.lock_snapshot,
        )?;
        let event_id = Uuid::now_v7();
        let event = encrypt_json(
            &key,
            input.session_id,
            &event_id.to_string(),
            1,
            "session_created",
            &input.initial_event_payload,
        )?;
        let outcome = input.command_outcome.clone();
        let outcome_json = serde_json::to_string(&outcome)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO sessions (
                session_id, key_id, state, created_at,
                configuration_nonce, configuration_ciphertext,
                lock_nonce, lock_ciphertext
            ) VALUES (?1, ?2, 'ready', ?3, ?4, ?5, ?6, ?7)",
            params![
                input.session_id.to_string(),
                key_id,
                format_time(input.occurred_at),
                configuration.nonce,
                configuration.ciphertext,
                lock.nonce,
                lock.ciphertext
            ],
        )?;
        insert_event(
            &transaction,
            &EventInput {
                event_id,
                session_id: input.session_id,
                occurred_at: input.occurred_at,
                kind: "session_created".to_owned(),
                causation_request_id: Some(input.request_id),
                attempt_id: None,
                effect_class: None,
                payload: Value::Null,
            },
            1,
            &key_id,
            &event,
        )?;
        transaction.execute(
            "INSERT INTO command_outcomes (
                scope, request_id, session_id, method, parameter_hash, outcome_json
             ) VALUES ('daemon', ?1, ?2, 'session.create', ?3, ?4)",
            params![
                input.request_id.to_string(),
                input.session_id.to_string(),
                parameter_hash,
                outcome_json
            ],
        )?;
        let removed = transaction.execute(
            "DELETE FROM session_creation_journal
             WHERE session_id = ?1 AND request_id = ?2
               AND key_id = ?3 AND state = 'key_created'",
            params![
                input.session_id.to_string(),
                input.request_id.to_string(),
                key_id
            ],
        )?;
        if removed != 1 {
            return Err(StorageError::StorageUnavailable(None));
        }
        transaction.commit()?;
        Ok(CommandOutcome::Recorded(outcome))
    }

    fn mark_creation_key_created(
        &self,
        input: &CreateSession,
        parameter_hash: &str,
        key_id: &str,
    ) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE session_creation_journal
             SET state = 'key_created'
             WHERE session_id = ?1 AND request_id = ?2
               AND parameter_hash = ?3 AND key_id = ?4 AND state = 'prepared'",
            params![
                input.session_id.to_string(),
                input.request_id.to_string(),
                parameter_hash,
                key_id
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::StorageUnavailable(None));
        }
        Ok(())
    }

    /// Reconciles only session creations proven by this database's journal.
    pub fn resume_session_creations(&mut self) -> Result<usize, StorageError> {
        let journals = {
            let mut statement = self.connection.prepare(
                "SELECT session_id, request_id, parameter_hash, key_id, state
                 FROM session_creation_journal ORDER BY session_id",
            )?;
            statement
                .query_map([], session_creation_journal_from_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let count = journals.len();
        for journal in journals {
            self.reconcile_creation(&journal)?;
        }
        Ok(count)
    }

    fn reconcile_create_retry(
        &mut self,
        session_id: Uuid,
        request_id: Uuid,
        parameter_hash: &str,
    ) -> Result<(), StorageError> {
        if let Some(journal) = self.creation_journal_by_request(request_id)? {
            if journal.parameter_hash != parameter_hash {
                return Err(StorageError::RequestConflict);
            }
            self.reconcile_creation(&journal)?;
        }
        if let Some(journal) = self.creation_journal_by_session(session_id)? {
            if journal.request_id != request_id || journal.parameter_hash != parameter_hash {
                return Err(StorageError::RequestConflict);
            }
            self.reconcile_creation(&journal)?;
        }
        Ok(())
    }

    fn reconcile_creation_by_request(&mut self, request_id: Uuid) -> Result<(), StorageError> {
        if let Some(journal) = self.creation_journal_by_request(request_id)? {
            self.reconcile_creation(&journal)?;
        }
        Ok(())
    }

    fn creation_journal_by_request(
        &self,
        request_id: Uuid,
    ) -> Result<Option<SessionCreationJournal>, StorageError> {
        self.connection
            .query_row(
                "SELECT session_id, request_id, parameter_hash, key_id, state
                 FROM session_creation_journal WHERE request_id = ?1",
                [request_id.to_string()],
                session_creation_journal_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    fn creation_journal_by_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<SessionCreationJournal>, StorageError> {
        self.connection
            .query_row(
                "SELECT session_id, request_id, parameter_hash, key_id, state
                 FROM session_creation_journal WHERE session_id = ?1",
                [session_id.to_string()],
                session_creation_journal_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    fn reconcile_creation(&mut self, journal: &SessionCreationJournal) -> Result<(), StorageError> {
        if !self
            .keys
            .owns_session_key(journal.session_id, &journal.key_id)
            || !matches!(journal.state.as_str(), "prepared" | "key_created")
        {
            return Err(StorageError::StorageUnavailable(None));
        }
        let committed_key_id = self
            .connection
            .query_row(
                "SELECT key_id FROM sessions WHERE session_id = ?1",
                [journal.session_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(committed_key_id) = committed_key_id {
            if committed_key_id != journal.key_id {
                return Err(StorageError::StorageUnavailable(None));
            }
        } else {
            self.keys.destroy_session_key(journal.session_id)?;
        }
        let removed = self.connection.execute(
            "DELETE FROM session_creation_journal
             WHERE session_id = ?1 AND request_id = ?2
               AND parameter_hash = ?3 AND key_id = ?4 AND state = ?5",
            params![
                journal.session_id.to_string(),
                journal.request_id.to_string(),
                journal.parameter_hash,
                journal.key_id,
                journal.state
            ],
        )?;
        if removed != 1 {
            return Err(StorageError::StorageUnavailable(None));
        }
        Ok(())
    }

    /// Loads redacted paid-inference spend micros for one session (0 if unset).
    pub fn load_session_spend_usd_micros(&self, session_id: Uuid) -> Result<u64, StorageError> {
        let value: i64 = self
            .connection
            .query_row(
                "SELECT spend_usd_micros FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::SessionNotFound)?;
        u64::try_from(value).map_err(|_| StorageError::InvalidInput("spend_usd_micros overflow"))
    }

    /// Persists redacted paid-inference spend micros for one session.
    pub fn store_session_spend_usd_micros(
        &self,
        session_id: Uuid,
        spend_usd_micros: u64,
    ) -> Result<(), StorageError> {
        let spend = i64::try_from(spend_usd_micros)
            .map_err(|_| StorageError::InvalidInput("spend_usd_micros overflow"))?;
        let updated = self.connection.execute(
            "UPDATE sessions SET spend_usd_micros = ?1 WHERE session_id = ?2",
            params![spend, session_id.to_string()],
        )?;
        if updated != 1 {
            return Err(StorageError::SessionNotFound);
        }
        Ok(())
    }

    /// Loads all redacted per-session spend rows for ledger restore after restart.
    pub fn load_all_session_spends(&self) -> Result<Vec<(Uuid, u64)>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT session_id, spend_usd_micros FROM sessions WHERE spend_usd_micros > 0",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut spends = Vec::new();
        for row in rows {
            let (session_id, spend) = row?;
            let session_id = parse_uuid(&session_id)?;
            let spend = u64::try_from(spend)
                .map_err(|_| StorageError::InvalidInput("spend_usd_micros overflow"))?;
            spends.push((session_id, spend));
        }
        Ok(spends)
    }

    pub fn load_session(&self, session_id: Uuid) -> Result<StoredSession, StorageError> {
        let row = self
            .connection
            .query_row(
                "SELECT state, terminal_at,
                        configuration_nonce, configuration_ciphertext,
                        lock_nonce, lock_ciphertext
                 FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StorageError::SessionNotFound)?;
        let key = self.keys.session_key(session_id)?;
        let configuration_snapshot = decrypt_session_json(
            &key,
            session_id,
            "configuration",
            "configuration_snapshot",
            row.2,
            row.3,
        )?;
        let lock_snapshot =
            decrypt_session_json(&key, session_id, "lock", "session_lock", row.4, row.5)?;
        Ok(StoredSession {
            session_id,
            state: row.0,
            terminal_at: row.1.map(|value| parse_time(&value)).transpose()?,
            configuration_snapshot,
            lock_snapshot,
        })
    }

    pub fn load_session_state(&self, session_id: Uuid) -> Result<String, StorageError> {
        self.connection
            .query_row(
                "SELECT state FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::SessionNotFound)
    }

    pub fn load_sessions(&self) -> Result<Vec<StoredSession>, StorageError> {
        let session_ids = {
            let mut statement = self
                .connection
                .prepare("SELECT session_id FROM sessions ORDER BY session_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        session_ids
            .into_iter()
            .map(|session_id| self.load_session(parse_uuid(&session_id)?))
            .collect()
    }

    pub fn list_session_metadata(
        &self,
        limit: u16,
        before_session_id: Option<Uuid>,
    ) -> Result<SessionMetadataPage, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidInput(
                "session list limit must be between 1 and 100",
            ));
        }
        let before_session_id = before_session_id.map(|value| value.to_string());
        let query_limit = i64::from(limit) + 1;
        let mut statement = self.connection.prepare(
            "SELECT session_id, state, created_at, terminal_at
             FROM sessions
             WHERE (?1 IS NULL OR session_id < ?1)
             ORDER BY session_id DESC
             LIMIT ?2",
        )?;
        let mut sessions = statement
            .query_map(params![before_session_id, query_limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .map(|row| {
                let (session_id, state, created_at, terminal_at) = row?;
                Ok(StoredSessionMetadata {
                    session_id: parse_uuid(&session_id)?,
                    state,
                    created_at: parse_time(&created_at)?,
                    terminal_at: terminal_at.as_deref().map(parse_time).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let has_more = sessions.len() > usize::from(limit);
        sessions.truncate(usize::from(limit));
        let next_before_session_id = has_more.then(|| {
            sessions
                .last()
                .expect("a non-empty bounded page has a last session")
                .session_id
        });
        Ok(SessionMetadataPage {
            sessions,
            next_before_session_id,
        })
    }

    pub fn append_event(&mut self, input: &EventInput) -> Result<PersistedEvent, StorageError> {
        let key_id = self.session_key_id(input.session_id)?;
        let key = self.keys.session_key(input.session_id)?;
        let transaction = self.connection.transaction()?;
        let persisted = append_event_in_transaction(&transaction, input, &key_id, &key)?;
        transaction.commit()?;
        Ok(persisted)
    }

    pub fn append_events(
        &mut self,
        events: &[EventInput],
    ) -> Result<Vec<PersistedEvent>, StorageError> {
        let Some(first) = events.first() else {
            return Err(StorageError::InvalidInput(
                "atomic append requires at least one event",
            ));
        };
        if events
            .iter()
            .any(|event| event.session_id != first.session_id)
        {
            return Err(StorageError::InvalidInput(
                "atomic append cannot cross sessions",
            ));
        }
        let key_id = self.session_key_id(first.session_id)?;
        let key = self.keys.session_key(first.session_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut persisted = Vec::with_capacity(events.len());
        for event in events {
            persisted.push(append_event_in_transaction(
                &transaction,
                event,
                &key_id,
                &key,
            )?);
        }
        transaction.commit()?;
        Ok(persisted)
    }

    pub(crate) fn commit_port_command(
        &mut self,
        request_id: Uuid,
        record: &StoredCommandRecord,
        input: &EventInput,
    ) -> Result<StoredCommandCommit, StorageError> {
        validate_port_command(record)?;
        if record.method == "session.create" || input.causation_request_id != Some(request_id) {
            return Err(StorageError::InvalidInput(
                "transactional command scope is invalid",
            ));
        }

        let key_id = self.session_key_id(input.session_id)?;
        let key = self.keys.session_key(input.session_id)?;
        let scope = command_scope(Some(input.session_id));
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = lookup_stored_command(&transaction, &scope, request_id)? {
            if existing.method == record.method && existing.parameter_hash == record.parameter_hash
            {
                return Ok(StoredCommandCommit::Replayed(existing));
            }
            return Err(StorageError::RequestConflict);
        }

        let persisted = append_event_in_transaction(&transaction, input, &key_id, &key)?;
        insert_port_command(&transaction, Some(input.session_id), request_id, record)?;
        transaction.commit()?;
        Ok(StoredCommandCommit::Applied(persisted))
    }

    pub fn replay(
        &self,
        session_id: Uuid,
        after_sequence: u64,
    ) -> Result<Vec<PersistedEvent>, StorageError> {
        let after_sequence = i64::try_from(after_sequence)
            .map_err(|_| StorageError::InvalidInput("cursor exceeds SQLite range"))?;
        let key = self.keys.session_key(session_id)?;
        let mut statement = self.connection.prepare(
            "SELECT event_id, sequence, occurred_at, kind, causation_request_id,
                    attempt_id, effect_class, nonce, ciphertext
             FROM session_events
             WHERE session_id = ?1 AND sequence > ?2
             ORDER BY sequence",
        )?;
        let encrypted_rows = statement
            .query_map(params![session_id.to_string(), after_sequence], |row| {
                Ok(EncryptedEventRow {
                    event_id: row.get(0)?,
                    sequence: read_sequence(row, 1)?,
                    occurred_at: row.get(2)?,
                    kind: row.get(3)?,
                    causation_request_id: row.get(4)?,
                    attempt_id: row.get(5)?,
                    effect_class: row.get(6)?,
                    nonce: row.get(7)?,
                    ciphertext: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        encrypted_rows
            .into_iter()
            .map(|row| decrypt_event(&key, session_id, row))
            .collect()
    }

    pub fn record_command_outcome(
        &mut self,
        scope: &str,
        request_id: Uuid,
        session_id: Option<Uuid>,
        method: &str,
        parameters: &Value,
        outcome: &Value,
    ) -> Result<CommandOutcome, StorageError> {
        if scope != command_scope(session_id)
            || (session_id.is_none() && method != "session.create")
            || (session_id.is_some() && method == "session.create")
        {
            return Err(StorageError::InvalidInput(
                "command outcome scope is invalid",
            ));
        }
        let parameter_hash = canonical_hash(parameters)?;
        let transaction = self.connection.transaction()?;
        if let Some((recorded_method, recorded_hash, recorded_outcome)) =
            lookup_command(&transaction, scope, request_id)?
        {
            if recorded_method == method && recorded_hash == parameter_hash {
                return Ok(CommandOutcome::Replay(recorded_outcome));
            }
            return Err(StorageError::RequestConflict);
        }
        transaction.execute(
            "INSERT INTO command_outcomes (
                scope, request_id, session_id, method, parameter_hash, outcome_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope,
                request_id.to_string(),
                session_id.map(|id| id.to_string()),
                method,
                parameter_hash,
                serde_json::to_string(outcome)?
            ],
        )?;
        transaction.commit()?;
        Ok(CommandOutcome::Recorded(outcome.clone()))
    }

    pub fn lookup_command_outcome(
        &self,
        session_id: Option<Uuid>,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
    ) -> Result<Option<Value>, StorageError> {
        if (session_id.is_none() && method != "session.create")
            || (session_id.is_some() && method == "session.create")
            || method.is_empty()
            || method.len() > 128
        {
            return Err(StorageError::InvalidInput(
                "command outcome scope is invalid",
            ));
        }
        let parameter_hash = canonical_hash(parameters)?;
        let Some(existing) = self.lookup_port_command(session_id, request_id)? else {
            return Ok(None);
        };
        if existing.method != method || existing.parameter_hash != parameter_hash {
            return Err(StorageError::RequestConflict);
        }
        Ok(Some(serde_json::from_str(&existing.outcome)?))
    }

    pub fn commit_command_event(
        &mut self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        event: &EventInput,
    ) -> Result<CommandEventOutcome, StorageError> {
        match self.commit_command_events(
            request_id,
            method,
            parameters,
            outcome,
            std::slice::from_ref(event),
        )? {
            CommandEventsOutcome::Recorded {
                mut events,
                outcome,
            } => Ok(CommandEventOutcome::Recorded {
                event: events.pop().ok_or(StorageError::StorageUnavailable(None))?,
                outcome,
            }),
            CommandEventsOutcome::Replay(outcome) => Ok(CommandEventOutcome::Replay(outcome)),
        }
    }

    pub(crate) fn commit_export_command(
        &mut self,
        export_id: Uuid,
        request_id: Uuid,
        parameter_hash: &str,
        outcome: &Value,
        event: &EventInput,
    ) -> Result<CommandEventOutcome, StorageError> {
        if event.kind != "session_exported" || event.causation_request_id != Some(request_id) {
            return Err(StorageError::InvalidInput(
                "export command event is invalid",
            ));
        }
        let record = StoredCommandRecord {
            method: "session.export".to_owned(),
            parameter_hash: parameter_hash.to_owned(),
            outcome: serde_json::to_string(outcome)?,
        };
        validate_port_command(&record)?;
        let key_id = self.session_key_id(event.session_id)?;
        let key = self.keys.session_key(event.session_id)?;
        let scope = command_scope(Some(event.session_id));
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let journal_state: Option<String> = transaction
            .query_row(
                "SELECT state FROM export_journal
                 WHERE export_id = ?1 AND session_id = ?2
                   AND request_id = ?3 AND parameter_hash = ?4",
                params![
                    export_id.to_string(),
                    event.session_id.to_string(),
                    request_id.to_string(),
                    parameter_hash
                ],
                |row| row.get(0),
            )
            .optional()?;
        if journal_state.as_deref() != Some("published") {
            return Err(StorageError::StorageUnavailable(None));
        }
        if let Some(existing) = lookup_stored_command(&transaction, &scope, request_id)? {
            if existing.method != record.method || existing.parameter_hash != record.parameter_hash
            {
                return Err(StorageError::RequestConflict);
            }
            transaction.execute(
                "DELETE FROM export_journal WHERE export_id = ?1",
                [export_id.to_string()],
            )?;
            transaction.commit()?;
            return Ok(CommandEventOutcome::Replay(serde_json::from_str(
                &existing.outcome,
            )?));
        }
        let persisted = append_event_in_transaction(&transaction, event, &key_id, &key)?;
        insert_port_command(&transaction, Some(event.session_id), request_id, &record)?;
        let deleted = transaction.execute(
            "DELETE FROM export_journal WHERE export_id = ?1",
            [export_id.to_string()],
        )?;
        if deleted != 1 {
            return Err(StorageError::StorageUnavailable(None));
        }
        transaction.commit()?;
        Ok(CommandEventOutcome::Recorded {
            event: persisted,
            outcome: outcome.clone(),
        })
    }

    pub fn commit_command_events(
        &mut self,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        events: &[EventInput],
    ) -> Result<CommandEventsOutcome, StorageError> {
        let Some(first) = events.first() else {
            return Err(StorageError::InvalidInput(
                "transactional command requires at least one event",
            ));
        };
        if method == "session.create"
            || first.causation_request_id != Some(request_id)
            || events.iter().any(|event| {
                event.session_id != first.session_id || event.causation_request_id.is_none()
            })
        {
            return Err(StorageError::InvalidInput(
                "transactional command scope is invalid",
            ));
        }
        let record = StoredCommandRecord {
            method: method.to_owned(),
            parameter_hash: canonical_hash(parameters)?,
            outcome: serde_json::to_string(outcome)?,
        };
        validate_port_command(&record)?;
        let key_id = self.session_key_id(first.session_id)?;
        let key = self.keys.session_key(first.session_id)?;
        let scope = command_scope(Some(first.session_id));
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = lookup_stored_command(&transaction, &scope, request_id)? {
            if existing.method == record.method && existing.parameter_hash == record.parameter_hash
            {
                return Ok(CommandEventsOutcome::Replay(serde_json::from_str(
                    &existing.outcome,
                )?));
            }
            return Err(StorageError::RequestConflict);
        }
        let mut persisted = Vec::with_capacity(events.len());
        for event in events {
            persisted.push(append_event_in_transaction(
                &transaction,
                event,
                &key_id,
                &key,
            )?);
        }
        insert_port_command(&transaction, Some(first.session_id), request_id, &record)?;
        transaction.commit()?;
        Ok(CommandEventsOutcome::Recorded {
            events: persisted,
            outcome: outcome.clone(),
        })
    }

    pub(crate) fn lookup_port_command(
        &self,
        session_id: Option<Uuid>,
        request_id: Uuid,
    ) -> Result<Option<StoredCommandRecord>, StorageError> {
        let scope = command_scope(session_id);
        let primary: Option<(String, String, String)> = self
            .connection
            .query_row(
                "SELECT method, parameter_hash, outcome_json
                 FROM command_outcomes WHERE scope = ?1 AND request_id = ?2",
                params![scope, request_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((method, parameter_hash, outcome)) = primary {
            return Ok(Some(StoredCommandRecord {
                method,
                parameter_hash,
                outcome,
            }));
        }
        let tombstone = match session_id {
            None => self
                .connection
                .query_row(
                    "SELECT creation_method, creation_parameter_hash, creation_outcome_json
                     FROM deletion_tombstones WHERE creation_request_id = ?1",
                    [request_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?,
            Some(session_id) => self
                .connection
                .query_row(
                    "SELECT deletion_method, deletion_parameter_hash, deletion_outcome_json
                     FROM deletion_tombstones
                     WHERE session_id = ?1 AND deletion_request_id = ?2",
                    params![session_id.to_string(), request_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?,
        };
        Ok(
            tombstone.map(|(method, parameter_hash, outcome)| StoredCommandRecord {
                method,
                parameter_hash,
                outcome,
            }),
        )
    }

    pub(crate) fn record_port_command(
        &mut self,
        session_id: Option<Uuid>,
        request_id: Uuid,
        record: &StoredCommandRecord,
    ) -> Result<(), StorageError> {
        validate_port_command(record)?;
        if (session_id.is_none() && record.method != "session.create")
            || (session_id.is_some() && record.method == "session.create")
        {
            return Err(StorageError::InvalidInput(
                "command outcome scope is invalid",
            ));
        }
        if let Some(existing) = self.lookup_port_command(session_id, request_id)? {
            if existing.method == record.method && existing.parameter_hash == record.parameter_hash
            {
                return Ok(());
            }
            return Err(StorageError::RequestConflict);
        }
        if let Some(session_id) = session_id {
            let exists = self
                .connection
                .query_row(
                    "SELECT 1 FROM sessions WHERE session_id = ?1",
                    [session_id.to_string()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Err(StorageError::SessionNotFound);
            }
        }
        let scope = command_scope(session_id);
        self.connection.execute(
            "INSERT INTO command_outcomes (
                scope, request_id, session_id, method, parameter_hash, outcome_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope,
                request_id.to_string(),
                session_id.map(|id| id.to_string()),
                record.method,
                record.parameter_hash,
                record.outcome
            ],
        )?;
        Ok(())
    }

    pub fn recover_uncertain_attempts(
        &mut self,
        occurred_at: OffsetDateTime,
    ) -> Result<Vec<RecoveredAttempt>, StorageError> {
        let attempts = {
            let mut statement = self.connection.prepare(
                "SELECT DISTINCT started.session_id, started.attempt_id
                 FROM session_events AS started
                 WHERE started.kind = 'dispatch_started'
                   AND started.attempt_id IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM session_events AS finished
                       WHERE finished.session_id = started.session_id
                         AND finished.attempt_id = started.attempt_id
                         AND finished.kind IN (
                           'session_completed', 'session_failed', 'session_cancelled',
                           'session_abandoned', 'outcome_unknown', 'outcome_reconciled'
                         )
                   )
                 ORDER BY started.session_id, started.attempt_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut recovered = Vec::with_capacity(attempts.len());
        for (session, attempt) in attempts {
            let session_id = parse_uuid(&session)?;
            let attempt_id = parse_uuid(&attempt)?;
            let event_id = Uuid::now_v7();
            self.append_event(&EventInput {
                event_id,
                session_id,
                occurred_at,
                kind: "outcome_unknown".to_owned(),
                causation_request_id: None,
                attempt_id: Some(attempt_id),
                effect_class: None,
                payload: serde_json::json!({
                    "attempt_id": attempt_id,
                    "reason": "daemon_recovery",
                    "reconciliation_options": ["retry", "accept_result", "abandon"]
                }),
            })?;
            recovered.push(RecoveredAttempt {
                session_id,
                attempt_id,
                event_id,
            });
        }
        Ok(recovered)
    }

    pub fn request_deletion(
        &mut self,
        session_id: Uuid,
        deletion_id: Uuid,
        request_id: Uuid,
        occurred_at: OffsetDateTime,
        actor: &str,
    ) -> Result<DeletionSummary, StorageError> {
        let deletion_parameters = serde_json::json!({
            "confirm_session_id": session_id
        });
        let deletion_parameter_hash = canonical_hash(&deletion_parameters)?;
        if let Some(summary) =
            self.replay_deletion_tombstone(session_id, request_id, &deletion_parameter_hash)?
        {
            return Ok(summary);
        }
        self.ensure_no_pending_export(session_id)?;

        let state: Option<String> = self
            .connection
            .query_row(
                "SELECT state FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let state = state.ok_or(StorageError::SessionNotFound)?;
        if !matches!(
            state.as_str(),
            "completed" | "failed" | "cancelled" | "abandoned" | "deleting"
        ) {
            return Err(StorageError::InvalidInput(
                "only terminal sessions may be deleted",
            ));
        }

        let existing_request: Option<String> = self
            .connection
            .query_row(
                "SELECT request_id FROM deletion_journal WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(recorded_request) = existing_request {
            if recorded_request != request_id.to_string() {
                return Err(StorageError::RequestConflict);
            }
            return self.finish_deletion(session_id);
        }

        let key_id = self.session_key_id(session_id)?;
        let key = self.keys.session_key(session_id)?;
        let event_id = Uuid::now_v7();
        let transaction = self.connection.transaction()?;
        let sequence = next_sequence(&transaction, session_id)?;
        let payload = serde_json::json!({
            "deletion_id": deletion_id,
            "actor": actor
        });
        let encrypted = encrypt_json(
            &key,
            session_id,
            &event_id.to_string(),
            sequence,
            "session_deletion_requested",
            &payload,
        )?;
        insert_event(
            &transaction,
            &EventInput {
                event_id,
                session_id,
                occurred_at,
                kind: "session_deletion_requested".to_owned(),
                causation_request_id: Some(request_id),
                attempt_id: None,
                effect_class: None,
                payload: Value::Null,
            },
            sequence,
            &key_id,
            &encrypted,
        )?;
        transaction.execute(
            "INSERT INTO deletion_journal (session_id, deletion_id, request_id)
             VALUES (?1, ?2, ?3)",
            params![
                session_id.to_string(),
                deletion_id.to_string(),
                request_id.to_string()
            ],
        )?;
        transaction.execute(
            "UPDATE sessions SET state = 'deleting' WHERE session_id = ?1",
            [session_id.to_string()],
        )?;
        transaction.commit()?;
        drop(key);
        self.finish_deletion(session_id)
    }

    pub fn ensure_session_deletable(&self, session_id: Uuid) -> Result<(), StorageError> {
        self.ensure_no_pending_export(session_id)
    }

    pub fn resume_deletions(&mut self) -> Result<Vec<DeletionSummary>, StorageError> {
        let session_ids = {
            let mut statement = self
                .connection
                .prepare("SELECT session_id FROM deletion_journal ORDER BY session_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        session_ids
            .into_iter()
            .map(|session_id| self.finish_deletion(parse_uuid(&session_id)?))
            .collect()
    }

    pub fn run_retention(
        &mut self,
        now: OffsetDateTime,
        retention_days: Option<u32>,
    ) -> Result<Vec<DeletionSummary>, StorageError> {
        let Some(days) = retention_days else {
            return Ok(Vec::new());
        };
        if days == 0 {
            return Err(StorageError::InvalidInput(
                "retention must be at least one day",
            ));
        }
        let threshold = now - Duration::days(i64::from(days));
        let terminal_sessions = {
            let mut statement = self.connection.prepare(
                "SELECT session_id, terminal_at FROM sessions
                 WHERE terminal_at IS NOT NULL
                   AND state IN ('completed', 'failed', 'cancelled', 'abandoned')
                 ORDER BY session_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut due_sessions = Vec::new();
        for (session, terminal) in terminal_sessions {
            if parse_time(&terminal)? <= threshold {
                due_sessions.push(session);
            }
        }
        let mut deleted = Vec::with_capacity(due_sessions.len());
        for session in due_sessions {
            deleted.push(self.request_deletion(
                parse_uuid(&session)?,
                Uuid::now_v7(),
                Uuid::now_v7(),
                now,
                "retention",
            )?);
        }
        Ok(deleted)
    }

    pub fn is_deleted(&self, session_id: Uuid) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM deletion_tombstones WHERE session_id = ?1",
                [session_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn replay_deletion_tombstone(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        parameter_hash: &str,
    ) -> Result<Option<DeletionSummary>, StorageError> {
        let tombstone: Option<(String, String, String)> = self
            .connection
            .query_row(
                "SELECT deletion_id, deletion_request_id, deletion_parameter_hash
                 FROM deletion_tombstones WHERE session_id = ?1",
                [session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((deletion_id, recorded_request, recorded_hash)) = tombstone else {
            return Ok(None);
        };
        if recorded_request != request_id.to_string() || recorded_hash != parameter_hash {
            return Err(StorageError::SessionNotFound);
        }
        Ok(Some(DeletionSummary {
            session_id,
            deletion_id: parse_uuid(&deletion_id)?,
            key_destroyed: true,
        }))
    }

    fn finish_deletion(&mut self, session_id: Uuid) -> Result<DeletionSummary, StorageError> {
        self.ensure_no_pending_export(session_id)?;
        let journal: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT deletion_id, request_id FROM deletion_journal WHERE session_id = ?1",
                [session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (deletion_id, request_id) = journal.ok_or(StorageError::SessionNotFound)?;
        let deletion_id = parse_uuid(&deletion_id)?;
        self.keys.destroy_session_key(session_id)?;

        let transaction = self.connection.transaction()?;
        let creation_command: Option<(String, String, String, String)> = transaction
            .query_row(
                "SELECT request_id, method, parameter_hash, outcome_json
                 FROM command_outcomes
                 WHERE session_id = ?1 AND method = 'session.create' LIMIT 1",
                [session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        transaction.execute(
            "DELETE FROM command_outcomes WHERE session_id = ?1",
            [session_id.to_string()],
        )?;
        transaction.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            [session_id.to_string()],
        )?;
        let deletion_parameter_hash = canonical_hash(&serde_json::json!({
            "confirm_session_id": session_id
        }))?;
        let deletion_outcome = serde_json::to_string(&serde_json::json!({
            "session_id": session_id,
            "deletion_id": deletion_id,
            "key_destroyed": true
        }))?;
        let (creation_request_id, creation_method, creation_hash, creation_outcome) =
            creation_command.map_or((None, None, None, None), |record| {
                (
                    Some(record.0),
                    Some(record.1),
                    Some(record.2),
                    Some(record.3),
                )
            });
        transaction.execute(
            "INSERT OR REPLACE INTO deletion_tombstones (
                session_id, deletion_id,
                creation_request_id, creation_method, creation_parameter_hash,
                creation_outcome_json, deletion_request_id, deletion_method,
                deletion_parameter_hash, deletion_outcome_json, key_destroyed
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'session.delete', ?8, ?9, 1)",
            params![
                session_id.to_string(),
                deletion_id.to_string(),
                creation_request_id,
                creation_method,
                creation_hash,
                creation_outcome,
                request_id,
                deletion_parameter_hash,
                deletion_outcome
            ],
        )?;
        transaction.execute(
            "DELETE FROM deletion_journal WHERE session_id = ?1",
            [session_id.to_string()],
        )?;
        transaction.commit()?;
        Ok(DeletionSummary {
            session_id,
            deletion_id,
            key_destroyed: true,
        })
    }

    fn ensure_no_pending_export(&self, session_id: Uuid) -> Result<(), StorageError> {
        let pending = self
            .connection
            .query_row(
                "SELECT 1 FROM export_journal WHERE session_id = ?1",
                [session_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if pending {
            return Err(StorageError::InvalidInput("session has a pending export"));
        }
        Ok(())
    }

    fn session_key_id(&self, session_id: Uuid) -> Result<String, StorageError> {
        self.connection
            .query_row(
                "SELECT key_id FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::SessionNotFound)
    }
}

#[derive(Debug)]
pub(crate) struct EncryptedEventRow {
    pub event_id: String,
    pub sequence: u64,
    pub occurred_at: String,
    pub kind: String,
    pub causation_request_id: Option<String>,
    pub attempt_id: Option<String>,
    pub effect_class: Option<String>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

fn session_creation_journal_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionCreationJournal> {
    let session_id = row.get::<_, String>(0)?;
    let request_id = row.get::<_, String>(1)?;
    Ok(SessionCreationJournal {
        session_id: Uuid::parse_str(&session_id).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        request_id: Uuid::parse_str(&request_id).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        parameter_hash: row.get(2)?,
        key_id: row.get(3)?,
        state: row.get(4)?,
    })
}

pub(crate) fn decrypt_event(
    key: &SecretKey,
    session_id: Uuid,
    row: EncryptedEventRow,
) -> Result<PersistedEvent, StorageError> {
    let plaintext = EncryptedPayload {
        nonce: row.nonce,
        ciphertext: row.ciphertext,
    }
    .decrypt(
        key,
        &AssociatedData {
            schema_version: 1,
            session_id,
            object_id: &row.event_id,
            sequence: row.sequence,
            kind: &row.kind,
        },
    )?;
    Ok(PersistedEvent {
        event_id: parse_uuid(&row.event_id)?,
        session_id,
        sequence: row.sequence,
        occurred_at: parse_time(&row.occurred_at)?,
        kind: row.kind,
        causation_request_id: row
            .causation_request_id
            .map(|id| parse_uuid(&id))
            .transpose()?,
        attempt_id: row.attempt_id.map(|id| parse_uuid(&id)).transpose()?,
        effect_class: row.effect_class,
        payload: serde_json::from_slice(&plaintext)?,
    })
}

fn insert_event(
    transaction: &Transaction<'_>,
    input: &EventInput,
    sequence: u64,
    key_id: &str,
    encrypted: &EncryptedPayload,
) -> rusqlite::Result<()> {
    let sequence = i64::try_from(sequence)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, i64::MAX))?;
    transaction.execute(
        "INSERT INTO session_events (
            event_id, session_id, sequence, occurred_at, kind,
            causation_request_id, attempt_id, effect_class, key_id, nonce, ciphertext
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            input.event_id.to_string(),
            input.session_id.to_string(),
            sequence,
            format_time(input.occurred_at),
            input.kind,
            input.causation_request_id.map(|id| id.to_string()),
            input.attempt_id.map(|id| id.to_string()),
            input.effect_class,
            key_id,
            encrypted.nonce,
            encrypted.ciphertext
        ],
    )?;
    Ok(())
}

fn append_event_in_transaction(
    transaction: &Transaction<'_>,
    input: &EventInput,
    key_id: &str,
    key: &SecretKey,
) -> Result<PersistedEvent, StorageError> {
    let sequence = next_sequence(transaction, input.session_id)?;
    let encrypted = encrypt_json(
        key,
        input.session_id,
        &input.event_id.to_string(),
        sequence,
        &input.kind,
        &input.payload,
    )?;
    insert_event(transaction, input, sequence, key_id, &encrypted)?;
    if input.kind == "session_redirected" {
        transaction.execute(
            "UPDATE sessions
             SET state = CASE
                 WHEN state = 'awaiting_clarification' THEN 'ready'
                 ELSE state
             END
             WHERE session_id = ?1",
            [input.session_id.to_string()],
        )?;
    } else if let Some(state) = state_for_kind(&input.kind) {
        let terminal_at = TERMINAL_KINDS
            .contains(&input.kind.as_str())
            .then(|| format_time(input.occurred_at));
        transaction.execute(
            "UPDATE sessions
             SET state = ?2, terminal_at = COALESCE(terminal_at, ?3)
             WHERE session_id = ?1",
            params![input.session_id.to_string(), state, terminal_at],
        )?;
    }
    Ok(PersistedEvent {
        event_id: input.event_id,
        session_id: input.session_id,
        sequence,
        occurred_at: input.occurred_at,
        kind: input.kind.clone(),
        causation_request_id: input.causation_request_id,
        attempt_id: input.attempt_id,
        effect_class: input.effect_class.clone(),
        payload: input.payload.clone(),
    })
}

fn read_sequence(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn next_sequence(transaction: &Transaction<'_>, session_id: Uuid) -> Result<u64, StorageError> {
    let next: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1
         FROM session_events WHERE session_id = ?1",
        [session_id.to_string()],
        |row| row.get(0),
    )?;
    u64::try_from(next).map_err(|_| StorageError::StorageUnavailable(None))
}

fn lookup_command(
    connection: &Connection,
    scope: &str,
    request_id: Uuid,
) -> Result<Option<(String, String, Value)>, StorageError> {
    let record: Option<(String, String, String)> = connection
        .query_row(
            "SELECT method, parameter_hash, outcome_json
             FROM command_outcomes WHERE scope = ?1 AND request_id = ?2",
            params![scope, request_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    record
        .map(|(method, hash, outcome)| Ok((method, hash, serde_json::from_str(&outcome)?)))
        .transpose()
}

fn lookup_stored_command(
    connection: &Connection,
    scope: &str,
    request_id: Uuid,
) -> Result<Option<StoredCommandRecord>, StorageError> {
    connection
        .query_row(
            "SELECT method, parameter_hash, outcome_json
             FROM command_outcomes WHERE scope = ?1 AND request_id = ?2",
            params![scope, request_id.to_string()],
            |row| {
                Ok(StoredCommandRecord {
                    method: row.get(0)?,
                    parameter_hash: row.get(1)?,
                    outcome: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(StorageError::from)
}

fn insert_port_command(
    connection: &Connection,
    session_id: Option<Uuid>,
    request_id: Uuid,
    record: &StoredCommandRecord,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO command_outcomes (
            scope, request_id, session_id, method, parameter_hash, outcome_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            command_scope(session_id),
            request_id.to_string(),
            session_id.map(|id| id.to_string()),
            record.method,
            record.parameter_hash,
            record.outcome
        ],
    )?;
    Ok(())
}

fn encrypt_json(
    key: &SecretKey,
    session_id: Uuid,
    object_id: &str,
    sequence: u64,
    kind: &str,
    value: &Value,
) -> Result<EncryptedPayload, StorageError> {
    let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
    EncryptedPayload::encrypt(
        key,
        &plaintext,
        &AssociatedData {
            schema_version: 1,
            session_id,
            object_id,
            sequence,
            kind,
        },
    )
}

fn decrypt_session_json(
    key: &SecretKey,
    session_id: Uuid,
    object_id: &str,
    kind: &str,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
) -> Result<Value, StorageError> {
    let plaintext = EncryptedPayload { nonce, ciphertext }.decrypt(
        key,
        &AssociatedData {
            schema_version: 1,
            session_id,
            object_id,
            sequence: 0,
            kind,
        },
    )?;
    Ok(serde_json::from_slice(&plaintext)?)
}

pub(crate) fn canonical_hash(value: &Value) -> Result<String, StorageError> {
    let canonical = canonicalize(value);
    let encoded = serde_json::to_vec(&canonical)?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn command_scope(session_id: Option<Uuid>) -> String {
    session_id.map_or_else(|| "daemon".to_owned(), |id| format!("session/{id}"))
}

fn validate_port_command(record: &StoredCommandRecord) -> Result<(), StorageError> {
    let valid_hash = record.parameter_hash.len() == 64
        && record
            .parameter_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if record.method.is_empty()
        || record.method.len() > 128
        || !valid_hash
        || record.outcome.len() > 65_536
    {
        return Err(StorageError::InvalidInput(
            "recorded command is outside durable limits",
        ));
    }
    Ok(())
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

fn state_for_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "dispatch_started" | "session_resumed" => Some("running"),
        "clarification_requested" => Some("awaiting_clarification"),
        "approval_requested" => Some("awaiting_approval"),
        "session_paused" => Some("paused"),
        "cancel_requested" => Some("cancel_requested"),
        "outcome_unknown" => Some("outcome_unknown"),
        "session_completed" => Some("completed"),
        "session_failed" => Some("failed"),
        "session_cancelled" => Some("cancelled"),
        "session_abandoned" => Some("abandoned"),
        _ => None,
    }
}

fn format_time(time: OffsetDateTime) -> String {
    time.format(&Rfc3339)
        .expect("OffsetDateTime always formats as RFC 3339")
}

fn parse_time(value: &str) -> Result<OffsetDateTime, StorageError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn parse_uuid(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn validated_database_path(path: &Path) -> Result<std::path::PathBuf, StorageError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(StorageError::InvalidInput("database path must be absolute"));
    }
    if path.exists() && fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(StorageError::InvalidInput(
            "database path must not be a symlink",
        ));
    }
    let parent = path
        .parent()
        .ok_or(StorageError::InvalidInput("database requires a parent"))?;
    if fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(StorageError::InvalidInput(
            "database parent must not be a symlink",
        ));
    }
    let canonical_parent = parent.canonicalize()?;
    validate_private_directory(&canonical_parent)?;
    let file_name = path
        .file_name()
        .ok_or(StorageError::InvalidInput("database requires a file name"))?;
    Ok(canonical_parent.join(file_name))
}

#[cfg(unix)]
fn database_location_scope(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn database_location_scope(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(unix)]
fn validate_private_directory(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(StorageError::InvalidInput(
            "storage directory must be private",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_directory(_path: &Path) -> Result<(), StorageError> {
    Err(StorageError::InvalidInput("unsupported platform"))
}

#[cfg(unix)]
fn validate_private_storage_file(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let metadata = fs::symlink_metadata(path)?;
    let parent_metadata = fs::metadata(
        path.parent()
            .ok_or(StorageError::InvalidInput("database requires a parent"))?,
    )?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != parent_metadata.uid()
    {
        return Err(StorageError::InvalidInput(
            "storage file ownership or permissions are unsafe",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_storage_file(_path: &Path) -> Result<(), StorageError> {
    Err(StorageError::InvalidInput("unsupported platform"))
}

fn sidecar_paths(path: &Path) -> [std::path::PathBuf; 2] {
    let display = path.as_os_str().to_string_lossy();
    [
        std::path::PathBuf::from(format!("{display}-wal")),
        std::path::PathBuf::from(format!("{display}-shm")),
    ]
}

fn validate_existing_sidecars(path: &Path) -> Result<(), StorageError> {
    for sidecar in sidecar_paths(path) {
        if sidecar.exists() {
            validate_private_storage_file(&sidecar)?;
        }
    }
    Ok(())
}

fn secure_sidecars(path: &Path) -> Result<(), StorageError> {
    for sidecar in sidecar_paths(path) {
        if sidecar.exists() {
            set_private_permissions(&sidecar)?;
            validate_private_storage_file(&sidecar)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), StorageError> {
    Err(StorageError::InvalidInput("unsupported platform"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use rusqlite::params;
    use serde_json::json;
    use tempfile::TempDir;
    use time::OffsetDateTime;
    use uuid::Uuid;

    use super::{CreateSession, EventInput, SqliteStorage, canonical_hash};
    use crate::{CommandOutcome, KeyStore, MemoryKeyStore, StorageError};

    fn open_file(
        directory: &TempDir,
        name: &str,
        store: MemoryKeyStore,
    ) -> (SqliteStorage<MemoryKeyStore>, std::path::PathBuf) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                .expect("private test directory");
        }
        let path = directory.path().join(name);
        (
            SqliteStorage::open(&path, store).expect("open storage"),
            path,
        )
    }

    fn reopen(path: &Path, store: MemoryKeyStore) -> SqliteStorage<MemoryKeyStore> {
        SqliteStorage::open(path, store).expect("reopen storage")
    }

    fn creation_input(session_id: Uuid, request_id: Uuid) -> CreateSession {
        CreateSession {
            session_id,
            request_id,
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({"session_id": session_id, "state": "ready"}),
            configuration_snapshot: json!({"version": 1}),
            lock_snapshot: json!({"version": 1}),
            initial_event_payload: json!({"created": true}),
        }
    }

    fn prepare_creation(storage: &SqliteStorage<MemoryKeyStore>, input: &CreateSession) -> String {
        let parameter_hash =
            canonical_hash(&input.request_parameters).expect("canonical parameter hash");
        let key_id = storage.keys.session_key_id(input.session_id);
        storage
            .connection
            .execute(
                "INSERT INTO session_creation_journal (
                    session_id, request_id, parameter_hash, key_id, state
                 ) VALUES (?1, ?2, ?3, ?4, 'prepared')",
                params![
                    input.session_id.to_string(),
                    input.request_id.to_string(),
                    parameter_hash,
                    key_id
                ],
            )
            .expect("prepare creation");
        key_id
    }

    fn journal_count(storage: &SqliteStorage<MemoryKeyStore>) -> i64 {
        storage
            .connection
            .query_row("SELECT COUNT(*) FROM session_creation_journal", [], |row| {
                row.get(0)
            })
            .expect("journal count")
    }

    fn assert_retry_records(storage: &mut SqliteStorage<MemoryKeyStore>, input: &CreateSession) {
        assert!(matches!(
            storage.create_session(input).expect("retry session.create"),
            CommandOutcome::Recorded(_)
        ));
        assert_eq!(journal_count(storage), 0);
    }

    #[test]
    fn recovers_crash_after_durable_creation_intent_and_retries_same_ids() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (storage, path) = open_file(&directory, "intent.sqlite", store.clone());
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        let key_id = prepare_creation(&storage, &input);
        assert!(store.get(&key_id).expect("inspect key").is_none());
        drop(storage);

        let mut reopened = reopen(&path, store.clone());
        assert_eq!(reopened.resume_session_creations().expect("recover"), 1);
        assert!(store.get(&key_id).expect("inspect key").is_none());
        assert_retry_records(&mut reopened, &input);
    }

    #[test]
    fn recovers_crash_after_envelope_before_state_and_retries_same_ids() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (storage, path) = open_file(&directory, "envelope.sqlite", store.clone());
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        let key_id = prepare_creation(&storage, &input);
        assert_eq!(
            storage
                .keys
                .create_session_key(input.session_id)
                .expect("create envelope"),
            key_id
        );
        assert!(store.get(&key_id).expect("inspect key").is_some());
        drop(storage);

        let mut reopened = reopen(&path, store.clone());
        assert_eq!(reopened.resume_session_creations().expect("recover"), 1);
        assert!(store.get(&key_id).expect("inspect key").is_none());
        assert_retry_records(&mut reopened, &input);
    }

    #[test]
    fn recovers_crash_after_key_created_state_and_retries_same_ids() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (storage, path) = open_file(&directory, "key-created.sqlite", store.clone());
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        let key_id = prepare_creation(&storage, &input);
        storage
            .keys
            .create_session_key(input.session_id)
            .expect("create envelope");
        storage
            .connection
            .execute(
                "UPDATE session_creation_journal SET state = 'key_created'
                 WHERE session_id = ?1",
                [input.session_id.to_string()],
            )
            .expect("mark key created");
        drop(storage);

        let mut reopened = reopen(&path, store.clone());
        assert_eq!(reopened.resume_session_creations().expect("recover"), 1);
        assert!(store.get(&key_id).expect("inspect key").is_none());
        assert_retry_records(&mut reopened, &input);
    }

    #[test]
    fn committed_create_reopens_as_replay_without_journal_cleanup() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (mut storage, path) = open_file(&directory, "committed.sqlite", store.clone());
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        assert_retry_records(&mut storage, &input);
        let key_id = storage.keys.session_key_id(input.session_id);
        drop(storage);

        let mut reopened = reopen(&path, store.clone());
        assert_eq!(reopened.resume_session_creations().expect("recover"), 0);
        assert!(store.get(&key_id).expect("inspect key").is_some());
        assert!(matches!(
            reopened
                .create_session(&input)
                .expect("replay session.create"),
            CommandOutcome::Replay(_)
        ));
    }

    #[test]
    fn pending_create_rejects_changed_parameters_without_losing_original_retry() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (mut storage, _path) = open_file(&directory, "conflict.sqlite", store);
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        prepare_creation(&storage, &input);
        let mut conflict = input.clone();
        conflict.request_parameters = json!({"persistent": false});

        assert!(matches!(
            storage
                .create_session(&conflict)
                .expect_err("changed parameters must conflict"),
            StorageError::RequestConflict
        ));
        assert_eq!(journal_count(&storage), 1);
        assert_retry_records(&mut storage, &input);
    }

    #[test]
    fn recovery_never_deletes_same_session_envelope_owned_by_a_database_clone() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (initial, path_a) = open_file(&directory, "a.sqlite", store.clone());
        drop(initial);
        let path_b = directory.path().join("b.sqlite");
        fs::copy(&path_a, &path_b).expect("clone database");
        let storage_a = reopen(&path_a, store.clone());
        let mut storage_b = reopen(&path_b, store.clone());
        let identity_a: String = storage_a
            .connection
            .query_row(
                "SELECT storage_id FROM storage_identity WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("database A identity");
        let identity_b: String = storage_b
            .connection
            .query_row(
                "SELECT storage_id FROM storage_identity WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("database B identity");
        assert_eq!(identity_a, identity_b, "fixture must be a real DB clone");
        let session_id = Uuid::now_v7();
        let input_a = creation_input(session_id, Uuid::now_v7());
        let input_b = creation_input(session_id, Uuid::now_v7());
        let key_a = prepare_creation(&storage_a, &input_a);
        storage_a
            .keys
            .create_session_key(session_id)
            .expect("create database A envelope");
        storage_a
            .connection
            .execute(
                "UPDATE session_creation_journal SET state = 'key_created'
                 WHERE session_id = ?1",
                [session_id.to_string()],
            )
            .expect("mark database A key created");
        storage_b
            .create_session(&input_b)
            .expect("create database B session");
        let key_b = storage_b.keys.session_key_id(session_id);
        assert_ne!(key_a, key_b);
        assert!(store.get(&key_a).expect("inspect A key").is_some());
        assert!(store.get(&key_b).expect("inspect B key").is_some());
        drop(storage_a);

        let mut reopened_a = reopen(&path_a, store.clone());
        assert_eq!(
            reopened_a
                .resume_session_creations()
                .expect("recover database A"),
            1
        );
        assert!(store.get(&key_a).expect("inspect A key").is_none());
        assert!(store.get(&key_b).expect("inspect B key").is_some());
        assert_eq!(
            storage_b
                .load_session(session_id)
                .expect("database B session")
                .session_id,
            session_id
        );
    }

    #[test]
    fn copied_pending_journal_fails_closed_before_touching_original_envelope() {
        let directory = TempDir::new().expect("tempdir");
        let store = MemoryKeyStore::new();
        let (storage, path) = open_file(&directory, "original.sqlite", store.clone());
        let input = creation_input(Uuid::now_v7(), Uuid::now_v7());
        let original_key_id = prepare_creation(&storage, &input);
        storage
            .keys
            .create_session_key(input.session_id)
            .expect("create original envelope");
        storage
            .connection
            .execute(
                "UPDATE session_creation_journal SET state = 'key_created'
                 WHERE session_id = ?1",
                [input.session_id.to_string()],
            )
            .expect("mark original key created");
        drop(storage);

        let clone_path = directory.path().join("clone.sqlite");
        fs::copy(&path, &clone_path).expect("copy database");
        let mut clone = reopen(&clone_path, store.clone());
        assert!(clone.resume_session_creations().is_err());
        assert!(
            store
                .get(&original_key_id)
                .expect("inspect original envelope")
                .is_some()
        );

        let mut original = reopen(&path, store.clone());
        assert_eq!(
            original
                .resume_session_creations()
                .expect("recover original"),
            1
        );
        assert!(
            store
                .get(&original_key_id)
                .expect("inspect cleaned envelope")
                .is_none()
        );
    }

    #[test]
    fn transactional_command_rolls_back_event_and_state_when_outcome_insert_fails() {
        let mut storage =
            SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open storage");
        let session_id = Uuid::now_v7();
        storage
            .create_session(&CreateSession {
                session_id,
                request_id: Uuid::now_v7(),
                occurred_at: OffsetDateTime::UNIX_EPOCH,
                request_parameters: json!({"persistent": true}),
                command_outcome: json!({"session_id": session_id, "state": "ready"}),
                configuration_snapshot: json!({"version": 1}),
                lock_snapshot: json!({"version": 1}),
                initial_event_payload: json!({"created": true}),
            })
            .expect("create session");
        storage
            .connection
            .execute_batch(
                "CREATE TEMP TRIGGER fail_atomic_command_outcome
                 BEFORE INSERT ON command_outcomes
                 WHEN NEW.method = 'session.pause'
                 BEGIN
                     SELECT RAISE(ABORT, 'forced command outcome failure');
                 END;",
            )
            .expect("install failure trigger");
        let request_id = Uuid::now_v7();
        let events = [
            EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at: OffsetDateTime::UNIX_EPOCH,
                kind: "pause_requested".to_owned(),
                causation_request_id: Some(request_id),
                attempt_id: None,
                effect_class: None,
                payload: json!({"actor": "test"}),
            },
            EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at: OffsetDateTime::UNIX_EPOCH,
                kind: "session_paused".to_owned(),
                causation_request_id: Some(request_id),
                attempt_id: None,
                effect_class: None,
                payload: json!({"actor": "test"}),
            },
        ];

        let error = storage
            .commit_command_events(
                request_id,
                "session.pause",
                &json!({}),
                &json!({"state": "paused"}),
                &events,
            )
            .expect_err("trigger must abort transaction");
        assert!(matches!(error, StorageError::StorageUnavailable(_)));
        assert_eq!(
            storage
                .replay(session_id, 0)
                .expect("replay after failure")
                .len(),
            1
        );
        assert!(
            storage
                .lookup_port_command(Some(session_id), request_id)
                .expect("lookup after failure")
                .is_none()
        );
        let state: String = storage
            .connection
            .query_row(
                "SELECT state FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .expect("session state");
        assert_eq!(state, "ready");
    }
}
