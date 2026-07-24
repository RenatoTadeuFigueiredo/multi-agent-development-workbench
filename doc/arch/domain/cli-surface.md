# Headless CLI Surface

Feature 001 provides a non-interactive client for contract tests, scripts, and
diagnostics. It is not the Grok Build TUI and contains no routing, policy,
storage, or provider logic.

## Commands

| Command | Protocol method or behavior |
|---|---|
| `workbench daemon` | Start the local daemon on the platform endpoint. |
| `workbench config validate` | Resolve all non-session layers and report schema or reference errors. |
| `workbench config lock` | Atomically write the deterministic repository-scope lock. |
| `workbench session create` | `session.create`; print the new session ID and hashes. |
| `workbench session attach <id> --after <sequence>` | `session.attach`; replay and follow events. |
| `workbench prompt <id> [--role <role>] <text>` | `session.prompt`; optionally set the explicit target. |
| `workbench session pause\|resume\|cancel <id>` | Corresponding idempotent control command. |
| `workbench session redirect <id> <instruction>` | Append instruction to a paused or awaiting session. |
| `workbench session approve <id> <approval> --decision <grant\|deny>` | Record a protected-action decision through `session.approval.resolve`. |
| `workbench session reconcile <id> <attempt> <resolution>` | Resolve `outcome_unknown` as `retry`, `accept_result`, or `abandon`. |
| `workbench session export <id> --recipient <age-recipient> --output <path>` | Create one encrypted age v1 bundle. |
| `workbench session delete <id> --confirm <id>` | Start cryptographic deletion after exact-ID confirmation. |
| `workbench status [<id>]` | Use `status.get` or `session.get` to read health or folded state without provider dispatch. |

Prompt text may be read from standard input by passing `-`; it is never accepted
as a process environment variable. Commands use the installed configuration
unless an explicit configuration path is supplied.

`config validate` and `config lock` invoke the same daemon application services
in a bounded one-shot server composition. The CLI contains no independent
configuration, lock, routing, policy, storage, or provider decision logic.

## --json Contract

Human output is the default. `--json` emits one versioned JSON result for a
one-shot command and newline-delimited versioned event objects for
`session attach`. Diagnostics go to standard error; prompt and provider bodies
do not. JSON objects contain `schema_version`, `request_id`, `ok`, and exactly
one of `result` or `error`. Streaming objects additionally contain `event_id`,
`session_id`, and `sequence`.

## Stable Exit Codes

The stable codes are `0` success, `2` invalid input, `3` policy or approval
refusal, `4` unavailable provider or capability, `5` storage or key-store
failure, `6` uncertain outcome, `7` protocol incompatibility, and `70` redacted
internal failure. Stream disconnect caused by `client_lagged` exits `7` after
printing the final error object.

Every command supports `--request-id`; otherwise the client generates a UUIDv7.
SIGINT sends `session.cancel` only when the command owns an active prompt.
Disconnecting an observer never cancels the session.
