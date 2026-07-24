# Local Operations

Feature 001 runs one same-user Workbench daemon on macOS or Linux. It exposes
no TCP listener and performs no provider network call in the default profile.

## Runtime Layout

| Data | macOS | Linux |
|---|---|---|
| User configuration | `~/Library/Application Support/Workbench/config.yaml` | `${XDG_CONFIG_HOME:-~/.config}/workbench/config.yaml` |
| State directory | `~/Library/Application Support/Workbench/state/` | `${XDG_STATE_HOME:-~/.local/state}/workbench/` |
| IPC endpoint | `<user-temp>/workbench-<uid>/workbench.sock` | `${XDG_RUNTIME_DIR}/workbench/workbench.sock` |
| Root and session-key envelopes | macOS Keychain | Secret Service login collection |

State and endpoint directories use mode `0700`; state files, exports, and the
socket use mode `0600`. The non-secret repository lock follows repository
permissions. Startup rejects symlinks, unexpected ownership, broad permissions,
missing peer-credential support, and an occupied endpoint that cannot be proven
stale. An endpoint is stale only when its owner matches the current user, the
single-daemon lock can be acquired, and a connection attempt proves that no
listener is accepting; otherwise startup leaves it untouched and fails.

## Startup and Recovery

Startup acquires a single-daemon lock, validates configuration and the base
lock, opens the platform key store, migrates SQLite, and folds session events.
It does not accept clients until those steps succeed. Recovery marks every
started attempt without a definite terminal fact as `outcome_unknown` and
finishes durable deletion intents before exposing affected sessions.

`workbench status` reports protocol version, storage schema, key-store
availability, migration status, active-session counts, and bounded adapter
health without revealing paths, prompts, or credentials. A locked key store is
degraded and blocks persistent work; corruption or migration failure is fatal
and never triggers an empty-database fallback.

## Environment Variables

Linux honors `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, and `XDG_RUNTIME_DIR`; macOS
uses `TMPDIR` only to select the endpoint parent. Each override must be an
absolute path owned by the current user. Missing Linux config and state
variables use the paths in the runtime table; missing or unsafe runtime/temp
directories are fatal. Feature 001 accepts no credential, prompt, model, or
policy value from environment variables.

## Exit Codes

Daemon startup returns `0` after graceful shutdown, `2` for configuration or
lock failure, `5` for storage, migration, or key-store failure, `7` for an
unsupported platform or IPC contract, and `70` for a redacted internal error.
Headless command exit codes are governed by
`doc/arch/domain/cli-surface.md`.

## Backup, Export, and Removal

Copying SQLite alone is not a recoverable backup because session-key envelopes
remain in the platform store. Portable backup uses `workbench session export`
with explicit age recipients. Operators must never copy key-store secrets into
the repository.

Graceful shutdown stops accepting commands, persists accepted controls, waits
for the bounded shutdown deadline, and leaves unresolved external effects as
`outcome_unknown`. Uninstalling the binary does not delete sessions. Data
removal uses the session deletion command so key envelopes are destroyed before
ciphertext cleanup.
