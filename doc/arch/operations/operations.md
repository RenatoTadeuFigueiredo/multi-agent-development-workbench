# Local Operations

Feature 001 runs one same-user Workbench daemon on macOS or Linux. It exposes
no TCP listener and performs no provider network call in the default profile.

## Local Startup

Build and prepare the exact non-session configuration before starting the
daemon:

```bash
cargo build --workspace
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- daemon
```

The generated `.workbench/workbench.lock` includes the user layer and is local
to the workstation in this repository. Regenerate it after any built-in, user,
repository, or explicit configuration change. In another terminal,
`cargo run -p workbench-cli -- --json status` verifies protocol, migration,
key-store, and adapter health.

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
started attempt without a definite terminal fact as `outcome_unknown`,
reconciles journal-proven interrupted session creations, and finishes durable
encrypted-export journals before deletion intents can destroy their session
keys or clients can observe affected sessions.
The platform key store may contain envelopes belonging to another state
directory. Startup never deletes an envelope merely because it is absent from
the selected SQLite catalog; changing `XDG_STATE_HOME` therefore cannot erase
sessions retained under the previous state root. Key namespaces bind the
database's durable storage UUID to its canonical location, so a copied SQLite
file at another location cannot reuse or delete the original envelopes and
fails closed. Because this is the pre-release initial schema, development
databases created before the creation journal was introduced must be recreated.

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

An explicit configuration supplied with `--configuration` must be an absolute,
owner-controlled, non-symlink file. It is supported by `config validate`,
`config lock`, and `daemon`; live client commands use the configuration already
owned by the running daemon.

## Exit Codes

Daemon startup returns `0` after graceful shutdown, `2` for configuration or
lock failure, `5` for storage, migration, or key-store failure, `7` for an
unsupported platform or IPC contract, and `70` for a redacted internal error.
Headless command exit codes are governed by
`doc/arch/domain/cli-surface.md`.

The complete offline release gate is `make check`. The real credential-store
contract is intentionally separate:

```bash
make test-platform
```

Run it only in an expendable unlocked Keychain or Secret Service context.

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
