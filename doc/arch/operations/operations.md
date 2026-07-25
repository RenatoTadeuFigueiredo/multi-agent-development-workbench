# Local Operations

Workbench runs one same-user daemon per canonical workspace on macOS or Linux.
It exposes no TCP listener and performs no provider network call in the default
profile.

## Local Startup

Build and prepare the exact non-session configuration before starting the
daemon:

```bash
cargo build --workspace
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
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
| State directory | `~/Library/Application Support/Workbench/state/<workspace-id>/` | `${XDG_STATE_HOME:-~/.local/state}/workbench/<workspace-id>/` |
| IPC endpoint | `<user-temp>/workbench-<uid>/<workspace-id>.sock` | `${XDG_RUNTIME_DIR}/workbench/<workspace-id>.sock` |
| Root and session-key envelopes | macOS Keychain | Secret Service login collection |

`<workspace-id>` is the first 16 bytes of SHA-256 over the domain separator
`workbench-workspace-id-v1\0` followed by the UTF-8 canonical workspace path,
encoded as 32 lowercase hexadecimal characters. Canonical path aliases
therefore resolve the same daemon, while distinct workspaces have isolated
state, locks, and sockets.

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

## MCP gateway

The daemon owns a central MCP and tool gateway composed after lock validation.
Configured `mcp_servers` entries are pin-checked against the lock `mcps` map
(version + sha256). Empty MCP maps remain valid. Stdio servers launch with a
direct argv (no shell), private runtime working directory, piped stdio, and
workspace isolation. HTTP servers use the pinned endpoint identity; non-loopback
endpoints require TLS (composed via rustls with platform native roots), unpinned
redirects are rejected, and encoded responses default to an 8 MiB ceiling.

Tool dispatch intersects built-in, user, repository, session, role, workflow,
and effect-class policy. Unlisted tools are denied. Repository grants cannot
widen user-global denies. Protected effect classes and `approval: always`
operations emit `approval_requested` and wait for `session.approval.resolve`
before any external MCP call. Public tool events carry only bounded tool name,
lifecycle category, redacted outcome, and correlation identifiers.

On shutdown the gateway rejects new tool work, terminates supervised stdio
children, drains pipes, and reaps processes. Default automated suites use only
committed offline fakes (`fake_mcp`) with no network, credential-store, or
quota access.
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

## Grok ACP Provider

The Grok ACP adapter supervises the explicitly configured canonical executable
as `grok agent --no-leader stdio` in the canonical workspace and sets
`GROK_DISABLE_AUTOUPDATER=1`. It never invokes a shell or updater. Grok Build
owns authentication and credential storage; Workbench observes only bounded
authentication health and never reads, copies, logs, or exports account
material.

The repository lock pins `acp/1`, the bounded `--version` result, and
executable SHA-256. Before spawn, startup verifies the configured path,
ownership, permissions, private executable snapshot, and digest against the
lock. After spawn but before provider availability or prompt dispatch,
initialization verifies protocol, authentication state, required capabilities,
and optional `agentInfo.version` when advertised. An operator must stop the daemon, update
Grok Build outside Workbench, explicitly regenerate the lock, and pass both
stages before the new executable is accepted. Manual lock edits and
compatibility bypasses are unsupported.

Public adapter health exposes only `available` or `unavailable`.
Authentication-required, compatibility, spawn, and crash causes are returned
as bounded redacted startup or lifecycle errors.

A cancellation is definite only when the outstanding prompt ends with
`stopReason: cancelled`. Crash, EOF, malformed transport, error, process exit,
or a cancellation deadline after dispatch produces `outcome_unknown`, blocks
automation, and requires `workbench session reconcile`; the prompt is never
resent automatically. Shutdown closes child stdin, waits for bounded graceful
exit, terminates a survivor, drains its pipes, and reaps it.

Default tests use only the explicit fake ACP executable and consume no network,
account, installed Grok runtime, or provider quota. The optional live check is
handshake-only and remains separate from `make check`. Detailed configuration,
update, recovery, shutdown, redaction, and smoke instructions are in the
[Grok ACP provider runbook](../../../docs/operations/grok-acp-provider.md).

## Claude Code Provider

The Claude adapter supervises an explicitly configured official Claude Code
executable through `claude-code-stream-json/1`. Configuration must set
`type: subscription-cli`, `driver: claude-code`, and the real absolute
versioned executable. Symlinks and unsafe writable path components are
rejected.

`config lock` creates a private executable snapshot, runs bounded `--version`
and `auth status --json` probes, and pins version and SHA-256. The auth probe
accepts only an existing Claude subscription login. Workbench never offers
login, reads provider credentials, or invokes installation or update commands.
The operator authenticates and updates through the official CLI outside
Workbench, then explicitly re-locks.

Every prompt receives a fresh child in the canonical workspace. The fixed
profile enables bidirectional stream JSON and partial messages while disabling
the updater, provider transcript persistence, Chrome, slash commands, native
customizations, and inherited MCP configuration. Only `Read`, `Glob`, and
`Grep` are available. Native writes, shell, web, skills, plugins, subagents,
and interactive permission prompts are unavailable in this feature.

Raw stdout, stderr, thinking, usage, auth fields, tool arguments/results, and
provider identifiers never enter logs or durable events. Cancellation requires
both a correlated successful interrupt response and an aborted terminal
reason. Crash, malformed output, EOF, or incomplete cancellation after
dispatch becomes `outcome_unknown` and is never retried automatically.

Default validation uses the committed fake only. The ignored live smoke runs
auth, initialization, and interrupt-receipt checks without a user message;
inference requires separate operator authorization. Current `claude -p`,
Agent SDK, and third-party application use draws from subscription limits
under provider-controlled rules. See the
[Claude Code provider runbook](../../../docs/operations/claude-code-provider.md).

## Codex Provider

The Codex adapter supervises an explicitly configured official Codex CLI
executable through `codex-exec-jsonl/1`. Configuration must set
`type: subscription-cli`, `driver: codex`, and the real absolute versioned
executable. Symlinks and unsafe writable path components are rejected.

`config lock` creates a private executable snapshot, runs bounded `--version`
and `login status` probes, and pins version and SHA-256. The auth probe
accepts only an existing ChatGPT subscription login. Workbench never offers
login, reads `CODEX_HOME` credential files, or invokes installation or update
commands. The operator authenticates and updates through the official CLI
outside Workbench, then explicitly re-locks.

Every prompt receives a fresh child in the canonical workspace. The fixed
profile is `codex exec --json --ephemeral --sandbox read-only -C <workspace>
-m <model>`. Inherited API-key and OSS/local provider selectors are removed.
Native workspace-write, danger-full-access, approval bypass, MCP registration,
plugins, and session resume are unavailable in this feature.

Raw stdout, stderr, reasoning, usage, auth fields, command payloads, and
provider identifiers never enter logs or durable events. Cancellation is
confirmed only by a documented abort or cancelled terminal event before
reaping; otherwise the child is reaped unconfirmed. Crash, malformed output,
EOF, or incomplete cancellation after dispatch becomes `outcome_unknown` and
is never retried automatically.

Default validation uses the committed fake only. The ignored live smoke runs
login-status and version checks without a user message; inference requires
separate operator authorization. Programmatic `codex exec` eligibility and
charging remain provider-controlled.

## Legacy Global-State Migration

Releases before workspace-scoped state stored one database directly at
`<state-root>/workbench.sqlite3`. That database does not identify the workspace
that owns it, and storage key namespaces are bound to its canonical path.
Consequently, the new daemon never guesses an owner, copies the SQLite file, or
opens an empty workspace database while legacy state is present. Startup fails
with `LegacyStateRequiresMigration` when the legacy database exists and the
selected workspace database does not.

Use this transition procedure:

1. Keep the legacy database at its original path and run the previous Workbench
   release.
2. Export every session that must be retained to an explicit encrypted age
   bundle:

   ```bash
   workbench session export <session-id> \
     --recipient <age-recipient> \
     --output <session-id>.age
   ```

3. Verify and retain the encrypted bundles and the recipient private key
   outside the repository. Do not copy or move the SQLite database into a
   workspace directory; its path-bound key namespace makes such a copy
   unusable and unsafe.
4. This release has no supported `session import` command. If continued access
   to legacy sessions is required, keep using the previous release and do not
   remove the legacy database. Import bundles only through a future explicit,
   supported importer.
5. To accept a fresh workspace with no imported history, first verify the
   exports, stop the previous daemon, and archive the legacy database outside
   the active state root. Starting the new daemon then creates isolated
   workspace state. Retain the archive and exports until explicit import has
   completed.

This is deliberately fail-closed: an operator must choose between continued
legacy access and a fresh workspace after verified export. Deleting the legacy
database is never part of automatic startup or migration.

## Environment Variables

Linux honors `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, and `XDG_RUNTIME_DIR`; macOS
uses `TMPDIR` only to select the endpoint parent. Each override must be an
absolute path owned by the current user. Missing Linux config and state
variables use the paths in the runtime table; missing or unsafe runtime/temp
directories are fatal. Workbench accepts no credential, prompt, model, or
policy value from environment variables. The adapter-owned
`GROK_DISABLE_AUTOUPDATER=1` and `DISABLE_AUTOUPDATER=1` child settings are
fixed by Workbench and are not user configuration or secret channels. Claude
API-key, alternate-endpoint, and cloud-provider selector variables are removed
from the Claude child rather than accepted as configuration.

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
