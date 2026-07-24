use std::{collections::BTreeMap, fs, path::Path};

use serde_json::json;
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use workbench_config::{
    ConfigurationSnapshot, WorkbenchConfiguration, WorkbenchLock,
    model::{DataCollection, Privacy, Provider, ProviderType},
};
use workbench_storage::{
    CommandEventOutcome, CommandOutcome, CreateSession, EventInput, ExportCommand, KeyStore,
    MemoryKeyStore, SqliteStorage, recipient_fingerprints,
};

fn open_storage(
    directory: &TempDir,
    store: MemoryKeyStore,
) -> (SqliteStorage<MemoryKeyStore>, std::path::PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("private tempdir");
    }
    let path = directory.path().join("workbench.sqlite");
    (
        SqliteStorage::open(&path, store).expect("open storage"),
        path,
    )
}

fn create_session(
    storage: &mut SqliteStorage<MemoryKeyStore>,
    session_id: Uuid,
    now: OffsetDateTime,
) -> Uuid {
    let request_id = Uuid::now_v7();
    let (configuration_snapshot, session_lock, configuration_hash, lock_hash) =
        export_metadata_fixture();
    let outcome = storage
        .create_session(&CreateSession {
            session_id,
            request_id,
            occurred_at: now,
            request_parameters: json!({"persistent": true}),
            command_outcome: json!({
                "session_id": session_id,
                "state": "ready",
                "configuration_hash": configuration_hash.clone(),
                "lock_hash": lock_hash.clone(),
            }),
            configuration_snapshot,
            lock_snapshot: session_lock,
            initial_event_payload: json!({
                "configuration_hash": configuration_hash,
                "lock_hash": lock_hash,
            }),
        })
        .expect("create session");
    assert!(matches!(outcome, CommandOutcome::Recorded(_)));
    request_id
}

fn export_metadata_fixture() -> (serde_json::Value, serde_json::Value, String, String) {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "api-fixture".to_owned(),
        Provider {
            kind: ProviderType::Api,
            executable: None,
            credential_ref: Some("platform:export-test-credential".to_owned()),
            privacy: Some(Privacy {
                zero_data_retention: true,
                data_collection: DataCollection::Deny,
            }),
        },
    );
    let snapshot = ConfigurationSnapshot::create(&configuration, vec!["test".to_owned()])
        .expect("configuration snapshot");
    let base_lock =
        WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new()).expect("base lock");
    let session_lock =
        WorkbenchLock::session(&base_lock, &configuration, &snapshot).expect("session lock");
    let configuration_hash = snapshot.content_hash.clone();
    let lock_hash = session_lock.hash().expect("lock hash");
    (
        serde_json::to_value(snapshot).expect("snapshot JSON"),
        serde_json::to_value(session_lock).expect("lock JSON"),
        configuration_hash,
        lock_hash,
    )
}

fn append(
    storage: &mut SqliteStorage<MemoryKeyStore>,
    session_id: Uuid,
    now: OffsetDateTime,
    kind: &str,
    attempt_id: Option<Uuid>,
    payload: serde_json::Value,
) {
    storage
        .append_event(&EventInput {
            event_id: Uuid::now_v7(),
            session_id,
            occurred_at: now,
            kind: kind.to_owned(),
            causation_request_id: None,
            attempt_id,
            effect_class: None,
            payload,
        })
        .expect("append event");
}

fn export_command(
    session_id: Uuid,
    request_id: Uuid,
    export_id: Uuid,
    now: OffsetDateTime,
    output_path: &Path,
    age_recipients: &[String],
) -> ExportCommand {
    let fingerprints = recipient_fingerprints(age_recipients).expect("recipient fingerprints");
    let parameters = json!({
        "output_path": output_path.to_str().expect("UTF-8 output path"),
        "age_recipients": age_recipients,
    });
    ExportCommand {
        session_id,
        request_id,
        export_id,
        occurred_at: now,
        parameters,
        output_path: output_path.to_path_buf(),
        age_recipients: age_recipients.to_owned(),
        outcome: json!({
            "export_id": export_id,
            "format": "age-v1",
            "recipient_fingerprints": fingerprints.clone(),
        }),
        event_payload: json!({
            "export_id": export_id,
            "format": "age-v1",
            "recipient_fingerprints": fingerprints,
        }),
    }
}

#[test]
fn appends_replays_and_leaks_no_plaintext_to_sqlite_or_wal() {
    let directory = TempDir::new().expect("tempdir");
    let store = MemoryKeyStore::new();
    let (mut storage, database) = open_storage(&directory, store);
    let session_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    create_session(&mut storage, session_id, now);
    append(
        &mut storage,
        session_id,
        now,
        "input_recorded",
        None,
        json!({"content": "PROMPT-SECRET"}),
    );
    append(
        &mut storage,
        session_id,
        now,
        "provider_event",
        Some(Uuid::now_v7()),
        json!({"content": "MODEL-SECRET"}),
    );

    let replay = storage.replay(session_id, 1).expect("replay");
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].sequence, 2);
    assert_eq!(replay[0].payload["content"], "PROMPT-SECRET");

    for path in storage_files(&database) {
        let bytes = fs::read(path).expect("read storage file");
        for forbidden in [b"PROMPT-SECRET".as_slice(), b"MODEL-SECRET".as_slice()] {
            assert!(
                !bytes
                    .windows(forbidden.len())
                    .any(|window| window == forbidden),
                "plaintext leaked into persistent storage"
            );
        }
    }
}

#[test]
fn command_outcomes_replay_and_reject_conflicting_reuse() {
    let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open storage");
    let request_id = Uuid::now_v7();
    let first = storage
        .record_command_outcome(
            "daemon",
            request_id,
            None,
            "session.create",
            &json!({"a": 1, "b": 2}),
            &json!({"state": "paused"}),
        )
        .expect("record");
    let replay = storage
        .record_command_outcome(
            "daemon",
            request_id,
            None,
            "session.create",
            &json!({"b": 2, "a": 1}),
            &json!({"state": "ignored"}),
        )
        .expect("replay");
    assert!(matches!(first, CommandOutcome::Recorded(_)));
    assert_eq!(replay, CommandOutcome::Replay(json!({"state": "paused"})));
    assert!(
        storage
            .record_command_outcome(
                "daemon",
                request_id,
                None,
                "session.create",
                &json!({"a": 1, "b": 3}),
                &json!({})
            )
            .is_err()
    );
}

#[test]
fn session_metadata_listing_is_bounded_cursor_based_and_key_independent() {
    let store = MemoryKeyStore::new();
    let mut storage = SqliteStorage::open_in_memory(store.clone()).expect("open metadata storage");
    let now = OffsetDateTime::now_utc();
    let session_ids = (0..5)
        .map(|offset| {
            let session_id = Uuid::now_v7();
            create_session(&mut storage, session_id, now + Duration::seconds(offset));
            session_id
        })
        .collect::<Vec<_>>();
    let terminal_session_id = session_ids[2];
    let terminal_at = now + Duration::minutes(1);
    append(
        &mut storage,
        terminal_session_id,
        terminal_at,
        "session_completed",
        None,
        json!({"summary": "done"}),
    );

    let mut expected = session_ids.clone();
    expected.sort_by_key(|session_id| std::cmp::Reverse(session_id.to_string()));
    store.set_available(false);

    let first = storage
        .list_session_metadata(2, None)
        .expect("first metadata page without key store");
    assert_eq!(
        first
            .sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        expected[..2]
    );
    assert_eq!(first.next_before_session_id, Some(expected[1]));

    let second = storage
        .list_session_metadata(2, first.next_before_session_id)
        .expect("second metadata page");
    assert_eq!(
        second
            .sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        expected[2..4]
    );
    assert_eq!(second.next_before_session_id, Some(expected[3]));

    let third = storage
        .list_session_metadata(2, second.next_before_session_id)
        .expect("final metadata page");
    assert_eq!(
        third
            .sessions
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        expected[4..]
    );
    assert_eq!(third.next_before_session_id, None);

    let terminal = first
        .sessions
        .iter()
        .chain(&second.sessions)
        .chain(&third.sessions)
        .find(|session| session.session_id == terminal_session_id)
        .expect("terminal session metadata");
    assert_eq!(terminal.state, "completed");
    assert_eq!(terminal.terminal_at, Some(terminal_at));
    assert!(
        storage.list_session_metadata(0, None).is_err(),
        "zero limit must be rejected at the storage boundary"
    );
    assert!(
        storage.list_session_metadata(101, None).is_err(),
        "oversized limit must be rejected at the storage boundary"
    );

    store.set_available(true);
    for session_id in session_ids {
        let history = storage
            .replay(session_id, 0)
            .expect("listing does not mutate event history");
        let expected_events = usize::from(session_id == terminal_session_id) + 1;
        assert_eq!(history.len(), expected_events);
    }
}

#[test]
fn session_metadata_projection_matches_redirect_fold_semantics() {
    let mut storage =
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open metadata storage");
    let now = OffsetDateTime::now_utc();
    let clarified_session_id = Uuid::now_v7();
    create_session(&mut storage, clarified_session_id, now);
    append(
        &mut storage,
        clarified_session_id,
        now + Duration::seconds(1),
        "clarification_requested",
        None,
        json!({"question": "choose a role"}),
    );
    append(
        &mut storage,
        clarified_session_id,
        now + Duration::seconds(2),
        "session_redirected",
        None,
        json!({"instruction": "use the coordinator"}),
    );

    let paused_session_id = Uuid::now_v7();
    create_session(&mut storage, paused_session_id, now + Duration::seconds(3));
    append(
        &mut storage,
        paused_session_id,
        now + Duration::seconds(4),
        "session_paused",
        None,
        json!({}),
    );
    append(
        &mut storage,
        paused_session_id,
        now + Duration::seconds(5),
        "session_redirected",
        None,
        json!({"instruction": "revise the plan"}),
    );

    let page = storage
        .list_session_metadata(100, None)
        .expect("redirected metadata");
    let clarified = page
        .sessions
        .iter()
        .find(|session| session.session_id == clarified_session_id)
        .expect("clarified session");
    assert_eq!(clarified.state, "ready");
    let paused = page
        .sessions
        .iter()
        .find(|session| session.session_id == paused_session_id)
        .expect("paused session");
    assert_eq!(paused.state, "paused");
}

#[test]
fn startup_recovery_marks_started_attempt_unknown_once() {
    let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open storage");
    let session_id = Uuid::now_v7();
    let attempt_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    create_session(&mut storage, session_id, now);
    append(
        &mut storage,
        session_id,
        now,
        "dispatch_started",
        Some(attempt_id),
        json!({"attempt_id": attempt_id}),
    );

    let recovered = storage
        .recover_uncertain_attempts(now + Duration::seconds(1))
        .expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].attempt_id, attempt_id);
    assert!(
        storage
            .recover_uncertain_attempts(now + Duration::seconds(2))
            .expect("idempotent recovery")
            .is_empty()
    );
}

#[test]
fn interrupted_deletion_resumes_after_key_store_returns() {
    let store = MemoryKeyStore::new();
    let mut storage = SqliteStorage::open_in_memory(store.clone()).expect("open storage");
    let session_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    create_session(&mut storage, session_id, now);
    append(
        &mut storage,
        session_id,
        now,
        "session_completed",
        None,
        json!({"summary": "done"}),
    );

    let deletion_id = Uuid::now_v7();
    store.set_available(false);
    assert!(
        storage
            .request_deletion(session_id, deletion_id, Uuid::now_v7(), now, "test")
            .is_err()
    );
    store.set_available(true);

    let resumed = storage.resume_deletions().expect("resume deletion");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].deletion_id, deletion_id);
    assert!(storage.is_deleted(session_id).expect("tombstone"));
    assert!(storage.replay(session_id, 0).is_err());
    assert!(
        store
            .list("workbench/storage/")
            .expect("list keys")
            .iter()
            .filter(|key_id| key_id.ends_with(&format!("/session/{session_id}/v1")))
            .count()
            == 0
    );
}

#[test]
fn completed_deletion_replays_by_request_id_from_tombstone() {
    let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open storage");
    let session_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    create_session(&mut storage, session_id, now);
    append(
        &mut storage,
        session_id,
        now,
        "session_completed",
        None,
        json!({"summary": "done"}),
    );
    let deletion_id = Uuid::now_v7();
    let request_id = Uuid::now_v7();
    let first = storage
        .request_deletion(session_id, deletion_id, request_id, now, "test")
        .expect("delete");
    let replay = storage
        .request_deletion(session_id, Uuid::now_v7(), request_id, now, "test")
        .expect("replay");
    assert_eq!(replay, first);
    assert!(
        storage
            .request_deletion(session_id, deletion_id, Uuid::now_v7(), now, "test")
            .is_err()
    );
}

#[test]
fn retention_deletes_only_due_terminal_sessions() {
    let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("open storage");
    let now = OffsetDateTime::now_utc();
    let due = Uuid::now_v7();
    let active = Uuid::now_v7();
    create_session(&mut storage, due, now - Duration::days(3));
    create_session(&mut storage, active, now - Duration::days(3));
    append(
        &mut storage,
        due,
        now - Duration::days(2),
        "session_completed",
        None,
        json!({"summary": "done"}),
    );

    assert!(
        storage
            .run_retention(now, None)
            .expect("default retention")
            .is_empty()
    );
    let deleted = storage.run_retention(now, Some(1)).expect("retention");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].session_id, due);
    assert!(storage.replay(active, 0).is_ok());
}

#[test]
#[allow(clippy::too_many_lines)]
fn export_is_age_v1_canonical_ndjson_and_never_overwrites() {
    let directory = TempDir::new().expect("tempdir");
    let (mut storage, _database) = open_storage(&directory, MemoryKeyStore::new());
    let session_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    create_session(&mut storage, session_id, now);
    append(
        &mut storage,
        session_id,
        now,
        "input_recorded",
        None,
        json!({"content": "EXPORT-SECRET"}),
    );
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public().to_string();
    let output = directory.path().join("session.age");

    let request_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let command = export_command(
        session_id,
        request_id,
        export_id,
        now,
        &output,
        std::slice::from_ref(&recipient),
    );
    let first = storage.execute_export(&command).expect("export");
    let CommandEventOutcome::Recorded { event, outcome } = first else {
        panic!("first export must be recorded");
    };
    assert!(!event.payload.to_string().contains(&recipient));
    assert!(!outcome.to_string().contains(&recipient));
    assert!(
        outcome["recipient_fingerprints"][0]
            .as_str()
            .expect("fingerprint")
            .starts_with("blake3:")
    );
    let replay = storage.execute_export(&command).expect("replay");
    assert!(matches!(replay, CommandEventOutcome::Replay(_)));
    let conflicting = export_command(
        session_id,
        Uuid::now_v7(),
        Uuid::now_v7(),
        now,
        &output,
        std::slice::from_ref(&recipient),
    );
    assert!(storage.execute_export(&conflicting).is_err());
    let ciphertext = fs::read(&output).expect("read export");
    assert!(ciphertext.starts_with(b"age-encryption.org/v1\n"));
    assert!(!ciphertext.windows(13).any(|part| part == b"EXPORT-SECRET"));
    let mut plaintext = age::decrypt(&identity, &ciphertext).expect("decrypt export");
    let text = String::from_utf8(plaintext.clone()).expect("utf8");
    plaintext.fill(0);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 5);
    let records = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("NDJSON record"))
        .collect::<Vec<_>>();
    for (line, record) in lines.iter().zip(&records) {
        assert_eq!(
            *line,
            serde_json::to_string(record).expect("canonical record")
        );
    }
    let (_, _, configuration_hash, lock_hash) = export_metadata_fixture();
    assert_eq!(
        records[0],
        json!({
            "configuration_hash": configuration_hash.clone(),
            "event_count": 2,
            "format": "workbench.session-export",
            "lock_hash": lock_hash,
            "schema_version": 1,
            "session_id": session_id,
        })
    );
    assert_eq!(records[1]["schema_version"], 1);
    assert_eq!(records[1]["content_hash"], configuration_hash);
    assert_eq!(
        records[1]["configuration"]["providers"]["api-fixture"]["credential_ref"],
        "[redacted]"
    );
    assert_eq!(records[2]["scope"], "session");
    assert_eq!(
        records[2]["configuration"]["resolved_hash"],
        configuration_hash
    );
    assert_eq!(records[3]["sequence"], 1);
    assert_eq!(records[4]["sequence"], 2);
    assert_eq!(records[4]["payload"]["content"], "EXPORT-SECRET");
    let session_key_id = format!("session/{session_id}/v1");
    for forbidden in [
        "platform:export-test-credential",
        "workbench/storage-root/v1",
        "workbench/storage/",
        session_key_id.as_str(),
        "\"ciphertext\"",
        "\"key_id\"",
        "\"nonce\"",
        "\"root_key\"",
        "\"session_key\"",
        "\"wrapped_key\"",
    ] {
        assert!(
            !text.contains(forbidden),
            "export leaked forbidden key or credential material: {forbidden}"
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(output).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn failed_export_creates_no_partial_file() {
    let directory = TempDir::new().expect("tempdir");
    let (mut storage, _database) = open_storage(&directory, MemoryKeyStore::new());
    let identity = age::x25519::Identity::generate();
    let output = directory.path().join("missing.age");

    let command = export_command(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        OffsetDateTime::now_utc(),
        &output,
        &[identity.to_public().to_string()],
    );
    assert!(storage.execute_export(&command).is_err());
    assert!(!output.exists());
}

#[cfg(unix)]
#[test]
fn rejects_symlink_and_broad_storage_permissions() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let directory = TempDir::new().expect("tempdir");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let target = directory.path().join("target.sqlite");
    fs::write(&target, []).expect("target");
    let link = directory.path().join("link.sqlite");
    symlink(&target, &link).expect("symlink");
    assert!(SqliteStorage::open(&link, MemoryKeyStore::new()).is_err());

    let database = directory.path().join("database.sqlite");
    let storage = SqliteStorage::open(&database, MemoryKeyStore::new()).expect("secure database");
    drop(storage);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).expect("broaden database");
    assert!(SqliteStorage::open(&database, MemoryKeyStore::new()).is_err());

    let broad_directory = TempDir::new().expect("broad tempdir");
    fs::set_permissions(broad_directory.path(), fs::Permissions::from_mode(0o755))
        .expect("broaden directory");
    assert!(
        SqliteStorage::open(
            &broad_directory.path().join("database.sqlite"),
            MemoryKeyStore::new()
        )
        .is_err()
    );
}

fn storage_files(database: &Path) -> Vec<std::path::PathBuf> {
    let parent = database.parent().expect("database parent");
    fs::read_dir(parent)
        .expect("read database directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("workbench.sqlite"))
        })
        .collect()
}
