use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Component, Path, PathBuf},
};

use rusqlite::{OptionalExtension as _, params};
use rustix::process::getuid;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use workbench_config::{
    ConfigurationSnapshot, WorkbenchLock, lock::LockScope, snapshot::canonical_json,
};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    AssociatedData, CommandEventOutcome, EncryptedPayload, EventInput, KeyStore, PersistedEvent,
    SecretKey, SqliteStorage, StorageError,
    sqlite::{EncryptedEventRow, StoredCommandRecord, canonical_hash},
};

const EXPORT_METHOD: &str = "session.export";
const EXPORT_JOURNAL_KIND: &str = "export_journal";

#[derive(Debug, Clone)]
pub struct ExportCommand {
    pub session_id: Uuid,
    pub request_id: Uuid,
    pub export_id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub parameters: Value,
    pub output_path: PathBuf,
    pub age_recipients: Vec<String>,
    pub outcome: Value,
    pub event_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub session_id: Uuid,
    pub event_count: usize,
    pub format: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportJournalPayload {
    schema_version: u32,
    parameters: Value,
    output_path: String,
    age_recipients: Vec<String>,
    outcome: Value,
    event_id: Uuid,
    occurred_at: String,
    event_payload: Value,
}

#[derive(Debug, Clone)]
struct ExportJournal {
    export_id: Uuid,
    session_id: Uuid,
    request_id: Uuid,
    parameter_hash: String,
    state: ExportState,
    payload: ExportJournalPayload,
    fingerprint: Option<FileFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportState {
    Prepared,
    Staged,
    Published,
}

impl ExportState {
    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "staged" => Ok(Self::Staged),
            "published" => Ok(Self::Published),
            _ => Err(StorageError::StorageUnavailable(None)),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Staged => "staged",
            Self::Published => "published",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    blake3: String,
    size: u64,
    device: u64,
    inode: u64,
}

impl<K: KeyStore> SqliteStorage<K> {
    /// Completes an idempotent, crash-safe age export command.
    pub fn execute_export(
        &mut self,
        command: &ExportCommand,
    ) -> Result<CommandEventOutcome, StorageError> {
        validate_export_command(command)?;
        let parameter_hash = canonical_hash(&command.parameters)?;
        if let Some(existing) =
            self.lookup_port_command(Some(command.session_id), command.request_id)?
        {
            return replay_export_command(&existing, &parameter_hash);
        }
        let journal = self.load_or_create_export_journal(command, &parameter_hash)?;
        self.advance_export(journal)
    }

    /// Resumes every export journal before deletion recovery can destroy keys.
    pub fn resume_exports(&mut self) -> Result<Vec<PersistedEvent>, StorageError> {
        let export_ids = {
            let mut statement = self
                .connection
                .prepare("SELECT export_id FROM export_journal ORDER BY export_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut recovered = Vec::with_capacity(export_ids.len());
        for export_id in export_ids {
            let journal = self.load_export_journal(parse_uuid(&export_id)?)?;
            if let CommandEventOutcome::Recorded { event, .. } = self.advance_export(journal)? {
                recovered.push(event);
            }
        }
        Ok(recovered)
    }

    fn load_or_create_export_journal(
        &mut self,
        command: &ExportCommand,
        parameter_hash: &str,
    ) -> Result<ExportJournal, StorageError> {
        if let Some(export_id) = self
            .connection
            .query_row(
                "SELECT export_id FROM export_journal
                 WHERE session_id = ?1 AND request_id = ?2",
                params![
                    command.session_id.to_string(),
                    command.request_id.to_string()
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let journal = self.load_export_journal(parse_uuid(&export_id)?)?;
            if journal.parameter_hash != parameter_hash {
                return Err(StorageError::RequestConflict);
            }
            return Ok(journal);
        }

        let state: Option<String> = self
            .connection
            .query_row(
                "SELECT state FROM sessions WHERE session_id = ?1",
                [command.session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let state = state.ok_or(StorageError::SessionNotFound)?;
        if state == "deleting" || self.has_deletion_journal(command.session_id)? {
            return Err(StorageError::InvalidInput(
                "session deletion prevents export",
            ));
        }

        let output_path = normalized_output_path(&command.output_path)?;
        require_absent(&output_path)?;
        let output_path = output_path
            .to_str()
            .ok_or(StorageError::UnsafeExportPath)?
            .to_owned();
        self.ensure_no_pending_target(&output_path)?;
        validate_recipients(&command.age_recipients)?;
        let payload = ExportJournalPayload {
            schema_version: 1,
            parameters: command.parameters.clone(),
            output_path,
            age_recipients: command.age_recipients.clone(),
            outcome: command.outcome.clone(),
            event_id: Uuid::now_v7(),
            occurred_at: format_time(command.occurred_at)?,
            event_payload: command.event_payload.clone(),
        };
        let key = self.keys.session_key(command.session_id)?;
        let encrypted =
            encrypt_journal_payload(&key, command.session_id, command.export_id, &payload)?;
        self.connection.execute(
            "INSERT INTO export_journal (
                export_id, session_id, request_id, parameter_hash, state,
                payload_nonce, payload_ciphertext
             ) VALUES (?1, ?2, ?3, ?4, 'prepared', ?5, ?6)",
            params![
                command.export_id.to_string(),
                command.session_id.to_string(),
                command.request_id.to_string(),
                parameter_hash,
                encrypted.nonce,
                encrypted.ciphertext
            ],
        )?;
        Ok(ExportJournal {
            export_id: command.export_id,
            session_id: command.session_id,
            request_id: command.request_id,
            parameter_hash: parameter_hash.to_owned(),
            state: ExportState::Prepared,
            payload,
            fingerprint: None,
        })
    }

    fn ensure_no_pending_target(&self, output_path: &str) -> Result<(), StorageError> {
        let export_ids = {
            let mut statement = self
                .connection
                .prepare("SELECT export_id FROM export_journal ORDER BY export_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for export_id in export_ids {
            if self
                .load_export_journal(parse_uuid(&export_id)?)?
                .payload
                .output_path
                == output_path
            {
                return Err(StorageError::UnsafeExportPath);
            }
        }
        Ok(())
    }

    fn load_export_journal(&self, export_id: Uuid) -> Result<ExportJournal, StorageError> {
        type JournalRow = (
            String,
            String,
            String,
            String,
            Vec<u8>,
            Vec<u8>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
        );
        let row: JournalRow = self
            .connection
            .query_row(
                "SELECT session_id, request_id, parameter_hash, state,
                        payload_nonce, payload_ciphertext, ciphertext_blake3,
                        ciphertext_size, staging_device, staging_inode
                 FROM export_journal WHERE export_id = ?1",
                [export_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StorageError::StorageUnavailable(None))?;
        let session_id = parse_uuid(&row.0)?;
        let request_id = parse_uuid(&row.1)?;
        let state = ExportState::parse(&row.3)?;
        let key = self.keys.session_key(session_id)?;
        let encrypted_payload = EncryptedPayload {
            nonce: row.4,
            ciphertext: row.5,
        };
        let payload = decrypt_journal_payload(&key, session_id, export_id, &encrypted_payload)?;
        validate_journal_payload(&payload, &row.2)?;
        let fingerprint = match (row.6, row.7, row.8, row.9) {
            (None, None, None, None) if state == ExportState::Prepared => None,
            (Some(blake3), Some(size), Some(device), Some(inode))
                if state != ExportState::Prepared =>
            {
                Some(FileFingerprint {
                    blake3,
                    size: u64::try_from(size)
                        .map_err(|_| StorageError::StorageUnavailable(None))?,
                    device: parse_u64(&device)?,
                    inode: parse_u64(&inode)?,
                })
            }
            _ => return Err(StorageError::StorageUnavailable(None)),
        };
        Ok(ExportJournal {
            export_id,
            session_id,
            request_id,
            parameter_hash: row.2,
            state,
            payload,
            fingerprint,
        })
    }

    fn advance_export(
        &mut self,
        mut journal: ExportJournal,
    ) -> Result<CommandEventOutcome, StorageError> {
        let output_path = normalized_output_path(Path::new(&journal.payload.output_path))?;
        if output_path.to_str() != Some(journal.payload.output_path.as_str()) {
            return Err(StorageError::UnsafeExportPath);
        }
        let staging_path = staging_path(&output_path, journal.export_id)?;
        let parent = output_path.parent().ok_or(StorageError::UnsafeExportPath)?;

        if journal.state == ExportState::Prepared {
            remove_partial_stage(&staging_path)?;
            self.write_age_bundle(
                journal.session_id,
                &staging_path,
                &journal.payload.age_recipients,
            )?;
            let fingerprint = fingerprint_file(&staging_path)?;
            sync_directory(parent)?;
            self.update_export_state(journal.export_id, ExportState::Staged, Some(&fingerprint))?;
            journal.state = ExportState::Staged;
            journal.fingerprint = Some(fingerprint);
        }

        let fingerprint = journal
            .fingerprint
            .as_ref()
            .ok_or(StorageError::StorageUnavailable(None))?;
        if journal.state == ExportState::Staged {
            publish_staged(&staging_path, &output_path, fingerprint)?;
            sync_directory(parent)?;
            self.update_export_state(journal.export_id, ExportState::Published, Some(fingerprint))?;
            journal.state = ExportState::Published;
        }

        validate_fingerprint(&output_path, fingerprint)?;
        cleanup_stage(&staging_path, fingerprint)?;
        sync_directory(parent)?;
        let event = EventInput {
            event_id: journal.payload.event_id,
            session_id: journal.session_id,
            occurred_at: parse_time(&journal.payload.occurred_at)?,
            kind: "session_exported".to_owned(),
            causation_request_id: Some(journal.request_id),
            attempt_id: None,
            effect_class: None,
            payload: journal.payload.event_payload,
        };
        self.commit_export_command(
            journal.export_id,
            journal.request_id,
            &journal.parameter_hash,
            &journal.payload.outcome,
            &event,
        )
    }

    fn update_export_state(
        &self,
        export_id: Uuid,
        state: ExportState,
        fingerprint: Option<&FileFingerprint>,
    ) -> Result<(), StorageError> {
        let (digest, size, device, inode) =
            fingerprint.map_or((None, None, None, None), |fingerprint| {
                (
                    Some(fingerprint.blake3.as_str()),
                    i64::try_from(fingerprint.size).ok(),
                    Some(fingerprint.device.to_string()),
                    Some(fingerprint.inode.to_string()),
                )
            });
        if fingerprint.is_some() && size.is_none() {
            return Err(StorageError::StorageUnavailable(None));
        }
        let changed = self.connection.execute(
            "UPDATE export_journal
             SET state = ?2, ciphertext_blake3 = ?3, ciphertext_size = ?4,
                 staging_device = ?5, staging_inode = ?6
             WHERE export_id = ?1",
            params![
                export_id.to_string(),
                state.as_str(),
                digest,
                size,
                device,
                inode
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::StorageUnavailable(None));
        }
        Ok(())
    }

    fn has_deletion_journal(&self, session_id: Uuid) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM deletion_journal WHERE session_id = ?1",
                [session_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn write_age_bundle(
        &self,
        session_id: Uuid,
        output_path: &Path,
        recipients: &[String],
    ) -> Result<ExportSummary, StorageError> {
        let recipients = parse_recipients(recipients)?;
        let encryptor =
            age::Encryptor::with_recipients(recipients.iter().map(|recipient| recipient as _))
                .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))?;
        let key = self.keys.session_key(session_id)?;
        let metadata = self.export_metadata(session_id, &key)?;
        let rows = self.export_rows(session_id)?;
        let result = (|| {
            let output = create_private_file(output_path)?;
            let buffered = BufWriter::new(output);
            let mut encrypted = encryptor
                .wrap_output(buffered)
                .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))?;

            write_json_record(
                &mut encrypted,
                &json!({
                    "configuration_hash": metadata.configuration_hash,
                    "event_count": rows.len(),
                    "format": "workbench.session-export",
                    "lock_hash": metadata.lock_hash,
                    "schema_version": 1,
                    "session_id": session_id,
                }),
            )?;
            write_json_record(&mut encrypted, &metadata.configuration_snapshot)?;
            write_json_record(&mut encrypted, &metadata.session_lock)?;

            for row in &rows {
                let plaintext = EncryptedPayload {
                    nonce: row.nonce.clone(),
                    ciphertext: row.ciphertext.clone(),
                }
                .decrypt(
                    &key,
                    &AssociatedData {
                        schema_version: 1,
                        session_id,
                        object_id: &row.event_id,
                        sequence: row.sequence,
                        kind: &row.kind,
                    },
                )?;
                let mut line = Zeroizing::new(Vec::with_capacity(plaintext.len() + 512));
                write!(
                    line,
                    "{{\"attempt_id\":{},\"causation_request_id\":{},\"effect_class\":{},\"event_id\":{},\"kind\":{},\"occurred_at\":{},\"payload\":",
                    json_optional(row.attempt_id.as_ref())?,
                    json_optional(row.causation_request_id.as_ref())?,
                    json_optional(row.effect_class.as_ref())?,
                    serde_json::to_string(&row.event_id)?,
                    serde_json::to_string(&row.kind)?,
                    serde_json::to_string(&row.occurred_at)?,
                )?;
                line.extend_from_slice(&plaintext);
                writeln!(
                    line,
                    ",\"sequence\":{},\"session_id\":\"{}\"}}",
                    row.sequence, session_id
                )?;
                encrypted.write_all(&line)?;
            }
            let mut buffered = encrypted
                .finish()
                .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))?;
            buffered.flush()?;
            buffered.get_ref().sync_all()?;
            Ok(ExportSummary {
                session_id,
                event_count: rows.len(),
                format: "age/v1",
            })
        })();
        if result.is_err() {
            let _ignored = fs::remove_file(output_path);
        }
        result
    }

    fn export_metadata(
        &self,
        session_id: Uuid,
        key: &SecretKey,
    ) -> Result<ExportMetadata, StorageError> {
        let encrypted = self
            .connection
            .query_row(
                "SELECT configuration_nonce, configuration_ciphertext,
                        lock_nonce, lock_ciphertext
                 FROM sessions WHERE session_id = ?1",
                [session_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StorageError::SessionNotFound)?;
        let configuration_plaintext = EncryptedPayload {
            nonce: encrypted.0,
            ciphertext: encrypted.1,
        }
        .decrypt(
            key,
            &AssociatedData {
                schema_version: 1,
                session_id,
                object_id: "configuration",
                sequence: 0,
                kind: "configuration_snapshot",
            },
        )?;
        let lock_plaintext = EncryptedPayload {
            nonce: encrypted.2,
            ciphertext: encrypted.3,
        }
        .decrypt(
            key,
            &AssociatedData {
                schema_version: 1,
                session_id,
                object_id: "lock",
                sequence: 0,
                kind: "session_lock",
            },
        )?;
        let configuration_snapshot: Value = serde_json::from_slice(&configuration_plaintext)?;
        let session_lock: Value = serde_json::from_slice(&lock_plaintext)?;
        validate_export_metadata(configuration_snapshot, session_lock)
    }

    fn export_rows(&self, session_id: Uuid) -> Result<Vec<EncryptedEventRow>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT event_id, sequence, occurred_at, kind, causation_request_id,
                    attempt_id, effect_class, nonce, ciphertext
             FROM session_events WHERE session_id = ?1 ORDER BY sequence",
        )?;
        Ok(statement
            .query_map(params![session_id.to_string()], |row| {
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
            .collect::<Result<Vec<_>, _>>()?)
    }
}

struct ExportMetadata {
    configuration_snapshot: Value,
    session_lock: Value,
    configuration_hash: String,
    lock_hash: String,
}

fn validate_export_command(command: &ExportCommand) -> Result<(), StorageError> {
    if command.event_payload.as_object().is_none() || command.outcome.as_object().is_none() {
        return Err(StorageError::InvalidInput(
            "export outcome and event payload must be objects",
        ));
    }
    let expected_parameters = json!({
        "age_recipients": command.age_recipients,
        "output_path": command.output_path.to_str().ok_or(StorageError::UnsafeExportPath)?
    });
    if canonical_hash(&expected_parameters)? != canonical_hash(&command.parameters)? {
        return Err(StorageError::InvalidInput(
            "export parameters do not match command",
        ));
    }
    Ok(())
}

fn validate_journal_payload(
    payload: &ExportJournalPayload,
    parameter_hash: &str,
) -> Result<(), StorageError> {
    if payload.schema_version != 1
        || payload.age_recipients.is_empty()
        || payload.outcome.as_object().is_none()
        || payload.event_payload.as_object().is_none()
    {
        return Err(StorageError::StorageUnavailable(None));
    }
    validate_recipients(&payload.age_recipients)?;
    if canonical_hash(&payload.parameters)? != parameter_hash {
        return Err(StorageError::AuthenticationFailed);
    }
    parse_time(&payload.occurred_at)?;
    Ok(())
}

fn replay_export_command(
    existing: &StoredCommandRecord,
    parameter_hash: &str,
) -> Result<CommandEventOutcome, StorageError> {
    if existing.method != EXPORT_METHOD || existing.parameter_hash != parameter_hash {
        return Err(StorageError::RequestConflict);
    }
    Ok(CommandEventOutcome::Replay(serde_json::from_str(
        &existing.outcome,
    )?))
}

fn encrypt_journal_payload(
    key: &SecretKey,
    session_id: Uuid,
    export_id: Uuid,
    payload: &ExportJournalPayload,
) -> Result<EncryptedPayload, StorageError> {
    let plaintext = Zeroizing::new(serde_json::to_vec(payload)?);
    EncryptedPayload::encrypt(
        key,
        &plaintext,
        &AssociatedData {
            schema_version: 1,
            session_id,
            object_id: export_id
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
            sequence: 0,
            kind: EXPORT_JOURNAL_KIND,
        },
    )
}

fn decrypt_journal_payload(
    key: &SecretKey,
    session_id: Uuid,
    export_id: Uuid,
    payload: &EncryptedPayload,
) -> Result<ExportJournalPayload, StorageError> {
    let object_id = export_id.to_string();
    let plaintext = payload.decrypt(
        key,
        &AssociatedData {
            schema_version: 1,
            session_id,
            object_id: &object_id,
            sequence: 0,
            kind: EXPORT_JOURNAL_KIND,
        },
    )?;
    Ok(serde_json::from_slice(&plaintext)?)
}

fn validate_export_metadata(
    configuration_snapshot: Value,
    session_lock: Value,
) -> Result<ExportMetadata, StorageError> {
    let snapshot: ConfigurationSnapshot = serde_json::from_value(configuration_snapshot.clone())?;
    let lock: WorkbenchLock = serde_json::from_value(session_lock.clone())?;
    let calculated_configuration_hash = blake3::hash(
        canonical_json(&snapshot.configuration)
            .map_err(|_| invalid_metadata())?
            .as_bytes(),
    )
    .to_hex()
    .to_string();
    if snapshot.schema_version != 1
        || snapshot.content_hash != calculated_configuration_hash
        || !contains_only_redacted_references(&snapshot.configuration)
        || lock.scope != LockScope::Session
        || lock.configuration.resolved_hash != snapshot.content_hash
        || lock.verify().is_err()
    {
        return Err(invalid_metadata());
    }
    let lock_hash = lock.hash().map_err(|_| invalid_metadata())?;
    Ok(ExportMetadata {
        configuration_snapshot,
        session_lock,
        configuration_hash: snapshot.content_hash,
        lock_hash,
    })
}

fn contains_only_redacted_references(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().all(|(key, value)| match key.as_str() {
            "credential_ref" | "executable" => value.as_str() == Some("[redacted]"),
            "ciphertext" | "key_id" | "nonce" | "root_key" | "session_key" | "wrapped_key" => false,
            _ => contains_only_redacted_references(value),
        }),
        Value::Array(values) => values.iter().all(contains_only_redacted_references),
        _ => true,
    }
}

fn invalid_metadata() -> StorageError {
    StorageError::StorageUnavailable(None)
}

fn write_json_record(writer: &mut impl Write, value: &impl Serialize) -> Result<(), StorageError> {
    let mut line = Zeroizing::new(serde_json::to_vec(value)?);
    line.push(b'\n');
    writer.write_all(&line)?;
    line.clear();
    Ok(())
}

fn read_sequence(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn json_optional(value: Option<&String>) -> Result<String, StorageError> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map(|value| value.unwrap_or_else(|| "null".to_owned()))
        .map_err(StorageError::from)
}

fn parse_recipients(recipients: &[String]) -> Result<Vec<age::x25519::Recipient>, StorageError> {
    if recipients.is_empty() {
        return Err(StorageError::InvalidInput(
            "at least one age recipient is required",
        ));
    }
    recipients
        .iter()
        .map(|value| {
            value
                .parse::<age::x25519::Recipient>()
                .map_err(|_| StorageError::InvalidInput("invalid age recipient"))
        })
        .collect()
}

fn validate_recipients(recipients: &[String]) -> Result<(), StorageError> {
    parse_recipients(recipients).map(|_| ())
}

pub fn recipient_fingerprints(recipients: &[String]) -> Result<Vec<String>, StorageError> {
    parse_recipients(recipients).map(|parsed| {
        parsed
            .into_iter()
            .map(|recipient| {
                format!(
                    "blake3:{}",
                    blake3::hash(recipient.to_string().as_bytes()).to_hex()
                )
            })
            .collect()
    })
}

fn normalized_output_path(path: &Path) -> Result<PathBuf, StorageError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(StorageError::UnsafeExportPath);
    }
    let parent = path.parent().ok_or(StorageError::UnsafeExportPath)?;
    if !parent.exists() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(StorageError::UnsafeExportPath);
    }
    let canonical_parent = parent.canonicalize()?;
    let file_name = path.file_name().ok_or(StorageError::UnsafeExportPath)?;
    Ok(canonical_parent.join(file_name))
}

fn require_absent(path: &Path) -> Result<(), StorageError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(StorageError::UnsafeExportPath),
        Err(error) => Err(error.into()),
    }
}

fn staging_path(output_path: &Path, export_id: Uuid) -> Result<PathBuf, StorageError> {
    let parent = output_path.parent().ok_or(StorageError::UnsafeExportPath)?;
    Ok(parent.join(format!(".workbench-export-{export_id}.age.part")))
}

fn remove_partial_stage(path: &Path) -> Result<(), StorageError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == getuid().as_raw()
                && metadata.permissions().mode().trailing_zeros() >= 6 =>
        {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(StorageError::UnsafeExportPath),
        Err(error) => Err(error.into()),
    }
}

fn publish_staged(
    staging_path: &Path,
    output_path: &Path,
    fingerprint: &FileFingerprint,
) -> Result<(), StorageError> {
    validate_fingerprint(staging_path, fingerprint)?;
    match fs::symlink_metadata(output_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::hard_link(staging_path, output_path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    StorageError::UnsafeExportPath
                } else {
                    StorageError::from(error)
                }
            })?;
        }
        Ok(_) => {}
        Err(error) => return Err(error.into()),
    }
    validate_fingerprint(output_path, fingerprint)
}

fn cleanup_stage(path: &Path, fingerprint: &FileFingerprint) -> Result<(), StorageError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => {
            validate_fingerprint(path, fingerprint)?;
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn fingerprint_file(path: &Path) -> Result<FileFingerprint, StorageError> {
    let file = open_private_file(path)?;
    let metadata = file.metadata()?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = Zeroizing::new(vec![0_u8; 64 * 1024]);
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    buffer.zeroize();
    Ok(FileFingerprint {
        blake3: hasher.finalize().to_hex().to_string(),
        size: metadata.len(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn validate_fingerprint(path: &Path, expected: &FileFingerprint) -> Result<(), StorageError> {
    if &fingerprint_file(path)? != expected {
        return Err(StorageError::UnsafeExportPath);
    }
    Ok(())
}

fn open_private_file(path: &Path) -> Result<File, StorageError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| StorageError::UnsafeExportPath)?;
    let metadata = file
        .metadata()
        .map_err(|_| StorageError::UnsafeExportPath)?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode().trailing_zeros() < 6
    {
        return Err(StorageError::UnsafeExportPath);
    }
    Ok(file)
}

fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn format_time(value: OffsetDateTime) -> Result<String, StorageError> {
    value
        .format(&Rfc3339)
        .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn parse_time(value: &str) -> Result<OffsetDateTime, StorageError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn parse_uuid(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn parse_u64(value: &str) -> Result<u64, StorageError> {
    value
        .parse()
        .map_err(|error| StorageError::StorageUnavailable(Some(Box::new(error))))
}

fn create_private_file(path: &Path) -> Result<fs::File, StorageError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StorageError::UnsafeExportPath
            } else {
                StorageError::from(error)
            }
        })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs::{self, OpenOptions},
        io::Write as _,
        os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
        path::Path,
    };

    use serde_json::json;
    use tempfile::TempDir;
    use time::OffsetDateTime;
    use uuid::Uuid;
    use workbench_config::{ConfigurationSnapshot, WorkbenchConfiguration, WorkbenchLock};

    use super::{
        ExportCommand, ExportState, cleanup_stage, fingerprint_file, normalized_output_path,
        publish_staged, recipient_fingerprints, staging_path, sync_directory,
    };
    use crate::{
        CommandEventOutcome, CreateSession, EventInput, KeyStore as _, MemoryKeyStore,
        SqliteStorage, sqlite::canonical_hash,
    };

    fn private_tempdir() -> TempDir {
        let directory = TempDir::new().expect("tempdir");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private tempdir");
        directory
    }

    fn open_storage(path: &Path, store: MemoryKeyStore) -> SqliteStorage<MemoryKeyStore> {
        SqliteStorage::open(path, store).expect("storage")
    }

    fn create_session(
        storage: &mut SqliteStorage<MemoryKeyStore>,
        session_id: Uuid,
        now: OffsetDateTime,
        terminal: bool,
    ) {
        let configuration = WorkbenchConfiguration::safe_builtins();
        let snapshot = ConfigurationSnapshot::create(&configuration, vec!["test".to_owned()])
            .expect("snapshot");
        let base = WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new())
            .expect("base lock");
        let lock = WorkbenchLock::session(&base, &configuration, &snapshot).expect("session lock");
        let configuration_hash = snapshot.content_hash.clone();
        let lock_hash = lock.hash().expect("lock hash");
        storage
            .create_session(&CreateSession {
                session_id,
                request_id: Uuid::now_v7(),
                occurred_at: now,
                request_parameters: json!({"persistent": true}),
                command_outcome: json!({"session_id": session_id, "state": "ready"}),
                configuration_snapshot: serde_json::to_value(snapshot).expect("snapshot JSON"),
                lock_snapshot: serde_json::to_value(lock).expect("lock JSON"),
                initial_event_payload: json!({
                    "configuration_hash": configuration_hash,
                    "lock_hash": lock_hash,
                }),
            })
            .expect("create session");
        if terminal {
            storage
                .append_event(&EventInput {
                    event_id: Uuid::now_v7(),
                    session_id,
                    occurred_at: now,
                    kind: "session_completed".to_owned(),
                    causation_request_id: None,
                    attempt_id: None,
                    effect_class: None,
                    payload: json!({"summary": "done"}),
                })
                .expect("terminal event");
        }
    }

    fn command(
        session_id: Uuid,
        now: OffsetDateTime,
        output_path: &Path,
        recipient: &str,
    ) -> ExportCommand {
        let export_id = Uuid::now_v7();
        let recipients = vec![recipient.to_owned()];
        let fingerprints = recipient_fingerprints(&recipients).expect("fingerprints");
        ExportCommand {
            session_id,
            request_id: Uuid::now_v7(),
            export_id,
            occurred_at: now,
            parameters: json!({
                "output_path": output_path.to_str().expect("UTF-8 path"),
                "age_recipients": recipients,
            }),
            output_path: output_path.to_path_buf(),
            age_recipients: recipients,
            outcome: json!({
                "export_id": export_id,
                "format": "age-v1",
                "recipient_fingerprints": fingerprints,
            }),
            event_payload: json!({
                "export_id": export_id,
                "format": "age-v1",
                "recipient_fingerprints": fingerprints,
                "test_marker": "JOURNAL-EVENT-SECRET",
            }),
        }
    }

    fn prepare(
        storage: &mut SqliteStorage<MemoryKeyStore>,
        command: &ExportCommand,
    ) -> super::ExportJournal {
        let parameter_hash = canonical_hash(&command.parameters).expect("parameter hash");
        storage
            .load_or_create_export_journal(command, &parameter_hash)
            .expect("prepare journal")
    }

    fn stage(
        storage: &mut SqliteStorage<MemoryKeyStore>,
        mut journal: super::ExportJournal,
    ) -> super::ExportJournal {
        let output =
            normalized_output_path(Path::new(&journal.payload.output_path)).expect("output path");
        let staging = staging_path(&output, journal.export_id).expect("staging path");
        storage
            .write_age_bundle(
                journal.session_id,
                &staging,
                &journal.payload.age_recipients,
            )
            .expect("write staged bundle");
        let fingerprint = fingerprint_file(&staging).expect("fingerprint");
        sync_directory(output.parent().expect("output parent")).expect("sync directory");
        storage
            .update_export_state(journal.export_id, ExportState::Staged, Some(&fingerprint))
            .expect("record staged");
        journal.state = ExportState::Staged;
        journal.fingerprint = Some(fingerprint);
        journal
    }

    fn publish(
        storage: &SqliteStorage<MemoryKeyStore>,
        mut journal: super::ExportJournal,
    ) -> super::ExportJournal {
        let output =
            normalized_output_path(Path::new(&journal.payload.output_path)).expect("output path");
        let staging = staging_path(&output, journal.export_id).expect("staging path");
        let fingerprint = journal.fingerprint.as_ref().expect("fingerprint");
        publish_staged(&staging, &output, fingerprint).expect("publish");
        sync_directory(output.parent().expect("output parent")).expect("sync directory");
        storage
            .update_export_state(journal.export_id, ExportState::Published, Some(fingerprint))
            .expect("record published");
        journal.state = ExportState::Published;
        journal
    }

    fn assert_recovered_once(storage: &mut SqliteStorage<MemoryKeyStore>, command: &ExportCommand) {
        let recovered = storage.resume_exports().expect("resume exports");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].kind, "session_exported");
        assert_eq!(
            storage
                .connection
                .query_row("SELECT COUNT(*) FROM export_journal", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("journal count"),
            0
        );
        assert_eq!(
            storage
                .replay(command.session_id, 0)
                .expect("history")
                .iter()
                .filter(|event| event.kind == "session_exported")
                .count(),
            1
        );
        let replay = storage.execute_export(command).expect("command replay");
        let CommandEventOutcome::Replay(outcome) = replay else {
            panic!("completed export must replay");
        };
        assert_eq!(outcome, command.outcome);
    }

    #[test]
    fn resumes_prepared_staged_and_published_boundaries_exactly_once() {
        for boundary in ["prepared", "staged", "published"] {
            let directory = private_tempdir();
            let database = directory.path().join("workbench.sqlite");
            let output = directory.path().join(format!("{boundary}.age"));
            let store = MemoryKeyStore::new();
            let session_id = Uuid::now_v7();
            let now = OffsetDateTime::now_utc();
            let identity = age::x25519::Identity::generate();
            let recipient = identity.to_public().to_string();
            let command = command(session_id, now, &output, &recipient);
            {
                let mut storage = open_storage(&database, store.clone());
                create_session(&mut storage, session_id, now, false);
                let mut journal = prepare(&mut storage, &command);
                if boundary == "prepared" {
                    let normalized = normalized_output_path(&output).expect("normalized output");
                    let staging =
                        staging_path(&normalized, command.export_id).expect("staging path");
                    let mut partial =
                        super::create_private_file(&staging).expect("partial staging file");
                    partial
                        .write_all(b"PARTIAL-AGE-CIPHERTEXT")
                        .expect("partial write");
                    partial.sync_all().expect("partial sync");
                    sync_directory(normalized.parent().expect("parent"))
                        .expect("partial directory sync");
                } else {
                    journal = stage(&mut storage, journal);
                }
                if boundary == "published" {
                    let _journal = publish(&storage, journal);
                }
            }
            let mut recovered = open_storage(&database, store.clone());
            assert_recovered_once(&mut recovered, &command);
            let ciphertext = fs::read(&output).expect("published ciphertext");
            assert!(ciphertext.starts_with(b"age-encryption.org/v1\n"));
            let plaintext = age::decrypt(&identity, &ciphertext).expect("decrypt");
            assert!(
                String::from_utf8(plaintext)
                    .expect("UTF-8")
                    .contains("\"format\":\"workbench.session-export\"")
            );
        }
    }

    #[test]
    fn journal_payload_is_encrypted_and_blocks_deletion_until_recovered() {
        let directory = private_tempdir();
        let database = directory.path().join("workbench.sqlite");
        let output = directory.path().join("pending-sensitive-path.age");
        let store = MemoryKeyStore::new();
        let session_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let command = command(session_id, now, &output, &recipient);
        let mut storage = open_storage(&database, store.clone());
        create_session(&mut storage, session_id, now, true);
        let _journal = prepare(&mut storage, &command);

        storage
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .expect("checkpoint");
        let database_bytes = fs::read(&database).expect("database bytes");
        for forbidden in [
            output.to_str().expect("UTF-8 path"),
            recipient.as_str(),
            "JOURNAL-EVENT-SECRET",
        ] {
            assert!(
                !database_bytes
                    .windows(forbidden.len())
                    .any(|window| window == forbidden.as_bytes()),
                "journal leaked plaintext: {forbidden}"
            );
        }
        assert!(storage.ensure_session_deletable(session_id).is_err());
        assert!(
            storage
                .request_deletion(session_id, Uuid::now_v7(), Uuid::now_v7(), now, "test",)
                .is_err()
        );
        assert!(
            store
                .list("workbench/storage/")
                .expect("keys")
                .iter()
                .filter(|key_id| key_id.ends_with(&format!("/session/{session_id}/v1")))
                .count()
                == 1
        );

        assert_recovered_once(&mut storage, &command);
        storage
            .request_deletion(session_id, Uuid::now_v7(), Uuid::now_v7(), now, "test")
            .expect("delete after export recovery");
    }

    #[test]
    fn tampered_or_colliding_final_fails_closed_without_overwrite() {
        for same_bytes_new_inode in [false, true] {
            let directory = private_tempdir();
            let database = directory.path().join("workbench.sqlite");
            let output = directory.path().join("collision.age");
            let store = MemoryKeyStore::new();
            let session_id = Uuid::now_v7();
            let now = OffsetDateTime::now_utc();
            let recipient = age::x25519::Identity::generate().to_public().to_string();
            let command = command(session_id, now, &output, &recipient);
            let mut storage = open_storage(&database, store);
            create_session(&mut storage, session_id, now, false);
            let journal = prepare(&mut storage, &command);
            let journal = stage(&mut storage, journal);
            let journal = publish(&storage, journal);
            let staging = staging_path(&output, journal.export_id).expect("staging");
            let replacement = if same_bytes_new_inode {
                fs::read(&staging).expect("staged ciphertext")
            } else {
                b"FOREIGN-CIPHERTEXT".to_vec()
            };
            fs::remove_file(&output).expect("unlink published name");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&output)
                .expect("replacement");
            file.write_all(&replacement).expect("replacement bytes");
            file.sync_all().expect("replacement sync");

            assert!(storage.resume_exports().is_err());
            assert_eq!(fs::read(&output).expect("foreign output"), replacement);
            assert_eq!(
                storage
                    .connection
                    .query_row("SELECT COUNT(*) FROM export_journal", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .expect("journal count"),
                1
            );
            assert_eq!(
                storage
                    .replay(session_id, 0)
                    .expect("history")
                    .iter()
                    .filter(|event| event.kind == "session_exported")
                    .count(),
                0
            );
            cleanup_stage(&staging, journal.fingerprint.as_ref().expect("fingerprint"))
                .expect("cleanup test staging");
        }
    }

    #[test]
    fn foreign_target_created_after_staging_is_never_overwritten() {
        let directory = private_tempdir();
        let database = directory.path().join("workbench.sqlite");
        let output = directory.path().join("foreign.age");
        let store = MemoryKeyStore::new();
        let session_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let command = command(session_id, now, &output, &recipient);
        let mut storage = open_storage(&database, store);
        create_session(&mut storage, session_id, now, false);
        let journal = prepare(&mut storage, &command);
        let _journal = stage(&mut storage, journal);
        let mut foreign = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&output)
            .expect("foreign target");
        foreign.write_all(b"FOREIGN-TARGET").expect("foreign bytes");
        foreign.sync_all().expect("foreign sync");

        assert!(storage.resume_exports().is_err());
        assert_eq!(
            fs::read(&output).expect("foreign output"),
            b"FOREIGN-TARGET"
        );
        assert_eq!(
            storage
                .connection
                .query_row(
                    "SELECT state FROM export_journal WHERE export_id = ?1",
                    [command.export_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .expect("journal state"),
            "staged"
        );
        assert!(
            storage
                .lookup_command_outcome(
                    Some(session_id),
                    command.request_id,
                    "session.export",
                    &command.parameters,
                )
                .expect("outcome lookup")
                .is_none()
        );
    }
}
