# Grok ACP Provider Operations

This runbook covers the supervised official Grok Build process used as an ACP
version 1 provider. The Grok-derived Workbench terminal is a separate
presentation client and is not covered here.

## Ownership and Launch Boundary

Grok Build owns login, token refresh, cookies, and its credential store.
Authenticate with the official Grok Build client before starting Workbench.
Workbench observes only a bounded authentication status and must never read,
copy, configure, log, or export Grok credentials.

Configure the canonical executable as an absolute path. Do not include
arguments in `executable`:

```yaml
providers:
  grok:
    type: acp
    executable: /absolute/canonical/path/to/grok
```

The target must be a regular executable owned by the current user, with no
symlink component or group/world-writable permission. Workbench invokes it
directly, without a shell, using this fixed child profile:

```text
argv: grok agent --no-leader stdio
cwd:  canonical workspace
env:  GROK_DISABLE_AUTOUPDATER=1
```

`--no-leader` prevents global leader configuration from sharing the supervised
session across workspaces. It does not sandbox the process or disable every
Grok-owned plugin, hook, skill, or MCP configuration. Review that provider
configuration before using confidential repositories.

## Lock, Validate, and Start

Create a lock only after the intended executable is installed and
authenticated:

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- daemon
```

In another terminal, verify bounded health:

```bash
cargo run -p workbench-cli -- --json status
```

The workstation-local lock pins `acp/1`, the version reported by the bounded
`--version` probe, and the executable SHA-256. Before spawn, Workbench verifies
the configured path, ownership, permissions, private executable snapshot, and
digest. After spawn but before provider availability or prompt dispatch, ACP
initialization verifies protocol, authentication state, required capabilities,
and optional `agentInfo.version` when advertised. Grok Build 0.2.111 omits
`agentInfo`; the locked probe version and executable digest remain
authoritative. Never repair either failure stage by editing the lock.

## Explicit Update and Rollback

Workbench never invokes the Grok updater, and the supervised child cannot
auto-update. To accept an update:

1. Finish or reconcile active attempts, then stop the workspace daemon.
2. Record the current Grok Build version and retain a recoverable copy or the
   vendor-supported rollback method.
3. Run the official update flow outside Workbench.
4. Regenerate the lock with `config lock`, run `config validate`, and start the
   daemon.
5. Confirm adapter health before dispatching a prompt.

Re-locking accepts a changed binary identity; it does not waive ACP
compatibility. Initialization still rejects an incompatible protocol or a
missing required capability. To roll back, stop the daemon, restore the
previous executable and matching lock, then start and check health again.

## Failure and Reconciliation

| Condition | Operator action |
|---|---|
| Adapter status is `unavailable` because authentication is required | Stop Workbench, complete the official Grok login flow, then restart and check health. Do not copy credentials into Workbench configuration. |
| Digest or version mismatch | Verify that the update was intentional. Re-lock only after review; otherwise restore the pinned executable. |
| Daemon startup reports provider incompatibility | Restore the previous binary or wait for adapter compatibility. Do not bypass the lock or protocol check. |
| Crash before dispatch | Correct installation or compatibility, then retry. No external attempt started. |
| EOF, crash, malformed output, or cancel timeout after dispatch | Treat the attempt as `outcome_unknown`; do not resend automatically. Inspect durable history and independently determine the provider-side outcome. |

The public `status` contract intentionally exposes only `available` and
`unavailable`. More specific authentication, compatibility, spawn, and
protocol categories are returned as bounded startup or command errors without
raw provider diagnostics.

Only a pending prompt response with `stopReason: cancelled` confirms
cancellation. A successful cancel write, acknowledgement, EOF, process exit,
error, or silence does not. After independent review, reconcile explicitly:

```bash
workbench session reconcile <session-id> <attempt-id> \
  <retry|accept-result|abandon>
```

Choose `retry` only after establishing that repeating the prompt is safe.

## Shutdown and Diagnostics

Stop the daemon normally. It stops accepting new adapter work, closes child
stdin, waits for a bounded graceful exit, terminates a survivor, drains the
pipes, and reaps the process. If Workbench is killed forcibly, confirm that no
orphaned provider process remains before restart and reconcile any active
attempt as uncertain.

Logs and telemetry may contain only adapter kind, lifecycle phase, bounded
health/outcome, attempt ID, and correlation ID. Never attach raw JSON-RPC,
stdout, stderr, environment values, prompts, model output, repository paths,
or provider session IDs to issues or release evidence.

## Offline Gate and Optional Live Smoke

The default tests use only the explicitly configured fake ACP executable:

```bash
make test-acp
make check
```

These commands must not inspect `PATH`, execute an installed `grok`, open a
network connection, require an account, or consume provider quota.

The optional live check is separate and handshake-only. Run the redacted
ignored test manually only on an approved workstation, using the configured
canonical executable rather than the updater-managed symlink:

```bash
WORKBENCH_GROK_EXECUTABLE=/absolute/canonical/path/to/grok \
  cargo test -p workbench-testkit --test feature_004 --locked -- \
  --ignored --exact \
  live_provider_runtime_initializes_the_digest_pinned_snapshot_without_a_prompt
```

The test runs the bounded version probe, committed-lock verification, private
snapshot, production supervisor, and one compatible `initialize` response. It
never sends `session/new` or `session/prompt`; it is not an inference test and
must not consume model quota. Record only sanitized pass/fail, the separately
probed version, and protocol/capability compatibility. Treat all raw child
output as sensitive.

See the
[Grok ACP supply-chain review](../security/grok-acp-supply-chain-review.md)
before approving an executable update.
