PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS storage_identity (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    storage_id TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY NOT NULL,
    key_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    terminal_at TEXT,
    configuration_nonce BLOB NOT NULL,
    configuration_ciphertext BLOB NOT NULL,
    lock_nonce BLOB NOT NULL,
    lock_ciphertext BLOB NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS session_events (
    event_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    occurred_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    causation_request_id TEXT,
    attempt_id TEXT,
    effect_class TEXT,
    key_id TEXT NOT NULL,
    nonce BLOB NOT NULL CHECK (length(nonce) = 24),
    ciphertext BLOB NOT NULL,
    UNIQUE (session_id, sequence)
) STRICT;

CREATE INDEX IF NOT EXISTS session_events_replay
    ON session_events(session_id, sequence);
CREATE INDEX IF NOT EXISTS session_events_attempt
    ON session_events(session_id, attempt_id, kind);
CREATE UNIQUE INDEX IF NOT EXISTS session_attempt_unique_fact
    ON session_events(session_id, attempt_id, kind)
    WHERE attempt_id IS NOT NULL
      AND kind IN (
        'dispatch_planned', 'dispatch_started', 'dispatch_acknowledged',
        'outcome_unknown', 'outcome_reconciled'
      );
CREATE UNIQUE INDEX IF NOT EXISTS session_attempt_single_terminal
    ON session_events(session_id, attempt_id)
    WHERE attempt_id IS NOT NULL
      AND kind IN (
        'session_completed', 'session_failed',
        'session_cancelled', 'session_abandoned'
      );

CREATE TABLE IF NOT EXISTS command_outcomes (
    scope TEXT NOT NULL,
    request_id TEXT NOT NULL,
    session_id TEXT,
    method TEXT NOT NULL,
    parameter_hash TEXT NOT NULL,
    outcome_json TEXT NOT NULL,
    PRIMARY KEY (scope, request_id)
) STRICT;

CREATE INDEX IF NOT EXISTS command_outcomes_session
    ON command_outcomes(session_id);

CREATE TABLE IF NOT EXISTS session_creation_journal (
    session_id TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL UNIQUE,
    parameter_hash TEXT NOT NULL,
    key_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'key_created'))
) STRICT;

CREATE TABLE IF NOT EXISTS artifacts (
    artifact_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    key_id TEXT NOT NULL,
    nonce BLOB NOT NULL CHECK (length(nonce) = 24),
    ciphertext BLOB NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS export_journal (
    export_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE RESTRICT,
    request_id TEXT NOT NULL,
    parameter_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'staged', 'published')),
    payload_nonce BLOB NOT NULL CHECK (length(payload_nonce) = 24),
    payload_ciphertext BLOB NOT NULL,
    ciphertext_blake3 TEXT,
    ciphertext_size INTEGER CHECK (ciphertext_size IS NULL OR ciphertext_size >= 0),
    staging_device TEXT,
    staging_inode TEXT,
    UNIQUE (session_id, request_id),
    CHECK (
        (state = 'prepared'
            AND ciphertext_blake3 IS NULL
            AND ciphertext_size IS NULL
            AND staging_device IS NULL
            AND staging_inode IS NULL)
        OR
        (state IN ('staged', 'published')
            AND ciphertext_blake3 IS NOT NULL
            AND ciphertext_size IS NOT NULL
            AND staging_device IS NOT NULL
            AND staging_inode IS NOT NULL)
    )
) STRICT;

CREATE INDEX IF NOT EXISTS export_journal_session
    ON export_journal(session_id);

CREATE TABLE IF NOT EXISTS deletion_journal (
    session_id TEXT PRIMARY KEY NOT NULL,
    deletion_id TEXT NOT NULL UNIQUE,
    request_id TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE IF NOT EXISTS deletion_tombstones (
    session_id TEXT PRIMARY KEY NOT NULL,
    deletion_id TEXT NOT NULL UNIQUE,
    creation_request_id TEXT,
    creation_method TEXT,
    creation_parameter_hash TEXT,
    creation_outcome_json TEXT,
    deletion_request_id TEXT NOT NULL,
    deletion_method TEXT NOT NULL,
    deletion_parameter_hash TEXT NOT NULL,
    deletion_outcome_json TEXT NOT NULL,
    key_destroyed INTEGER NOT NULL CHECK (key_destroyed = 1)
) STRICT;

PRAGMA user_version = 1;
