# Local Protocol Semantics

This document fixes the behavioral meaning of `workbench/1`. The AsyncAPI file
defines envelopes and wire limits; this document defines command preconditions,
payloads, results, events, retries, and reconciliation.

## Transport and Encoding

- One UTF-8 JSON object occupies one newline-delimited frame. Embedded newlines
  are JSON escapes. Duplicate object keys, invalid UTF-8, trailing data, and
  frames larger than 8 MiB are rejected; the limit counts encoded JSON bytes
  and excludes the terminating newline.
- Method names use dot notation. States, event kinds, and error codes use
  `snake_case`. Identifiers are UUIDv7 strings; sequence and cursor values are
  unsigned 64-bit integers.
- The endpoint parent directory is mode `0700` and the Unix socket is mode
  `0600`. The daemon verifies platform peer credentials and the endpoint owner
  before processing `initialize`; unsupported verification fails closed.
- Audit actors are derived as `local-user:<uid>` from the verified peer
  credential. `client_name`, request parameters, and provider output are never
  accepted as authoritative actor identities.
- The server emits one reply per parsed request. Session events are
  at-least-once and ordered by session sequence. Clients deduplicate by
  `event_id` and resume strictly after their last durable sequence.

## Command Idempotency

`request_id` is the durable idempotency key for state-changing commands. It is
daemon-scoped for `session.create` and otherwise scoped by session. Before
replying, the daemon stores the request ID, method, canonical parameter hash,
and redacted outcome. Repeating the same request returns that outcome without
repeating state changes or external effects. Reusing the ID within its scope
with a different method or parameter hash returns `invalid_request`.
`session.create` stores its request mapping and new session ID in the same
transaction.

`initialize`, `status.get`, `session.get`, and `session.attach` are read or
connection operations. They do not persist command outcomes and may be
reexecuted with a new connection to return current state.

Events caused directly by a command carry `causation_request_id`. Command
records remain for the session lifetime; deletion retains only the
daemon-scoped creation and deletion request IDs and their redacted outcomes in
the non-sensitive tombstone.

## Commands

All commands carry `protocol` and `request_id`. Commands other than
`initialize`, `status.get`, and `session.create` also carry `session_id`.

| Method | Required parameters | Allowed state and result |
|---|---|---|
| `initialize` | `client_name`, `client_version`, `supported_protocols[]` | Before any other command; returns selected protocol and limits. |
| `status.get` | None | Returns redacted daemon, protocol, storage, key-store, migration, and adapter health. |
| `session.create` | `persistent: true`; optional `configuration_overrides` | Creates a persistent session; returns session ID, configuration hash, lock hash, and `ready`. |
| `session.get` | None | Any non-deleted state; returns the current folded state without subscribing. |
| `session.attach` | `after_sequence` (zero allowed) | Any non-deleted state; returns current state and begins replay after the cursor. |
| `session.prompt` | Non-empty `text`; optional `explicit_target` | `ready`; records input before returning its ID and sequence. |
| `session.pause` | None | `running` or `pausing`; returns the durable control outcome. |
| `session.resume` | None | `paused`; returns the durable control outcome. |
| `session.redirect` | Non-empty `instruction` | `paused` or `awaiting_clarification`; appends instruction without rewriting history. |
| `session.cancel` | None | Any non-terminal state; returns `cancel_requested` or a previously recorded terminal outcome. |
| `session.approval.resolve` | `approval_id`, `decision` as `grant` or `deny` | `awaiting_approval`, or replay of the same recorded decision; records actor and decision before either dispatching or pausing. |
| `session.reconcile` | `attempt_id`, `resolution`; optional `evidence` | `outcome_unknown`; resolution is `retry`, `accept_result`, or `abandon`. |
| `session.export` | Absolute `output_path`, non-empty `age_recipients[]` | Any non-deleted state; writes one encrypted age v1 bundle with mode `0600`. |
| `session.delete` | `confirm_session_id` equal to `session_id` | Any terminal state; records encrypted intent and a non-sensitive recovery journal, destroys the platform-stored key envelope, evicts the in-memory key, then purges rows and artifacts. |

Unknown parameters are rejected. Paths are normalized and rejected if they are
relative, traverse through `..`, target a symlink, or overwrite an existing
file. Public protocol v1 has no plaintext export or ephemeral-session command.
Configured retention is measured from the first terminal event and invokes the
same deletion flow; active and `outcome_unknown` sessions never expire.

The decrypted age v1 payload is newline-delimited canonical JSON. Its first
record is a `workbench.session-export` manifest with `schema_version: 1`,
session ID, configuration hash, lock hash, and event count. A redacted
configuration snapshot, session lock, and events in ascending sequence follow.
Key material, credential values, platform key-store identifiers, and plaintext
temporary files are forbidden.

## Event Payloads

Every event includes the common AsyncAPI envelope and its causation request
when one exists. The `data` object contains only the fields below; content
fields are encrypted in persistent storage.

| Event kind | Required data |
|---|---|
| `session_created` | `configuration_hash`, `lock_hash` |
| `configuration_resolved` | `snapshot_hash`, `sources[]`, `role_mappings[]` |
| `input_recorded` | `input_id`, `content` |
| `routing_planned` | `intent`, `role`, `model_alias`, `provider`, `context_sources[]`, `tools[]`, `permission_scope[]`, `risk`, `confidence`, `selected_by` |
| `clarification_requested` | `question`, `reason` |
| `approval_requested` | `approval_id`, `action`, `risk`, `scope` |
| `approval_recorded` | `approval_id`, `actor`, `decision` |
| `dispatch_planned` | `attempt_id`, `effect_class`, `operation`, `idempotent` |
| `dispatch_started` | `attempt_id`, `adapter_session_id` when known |
| `dispatch_acknowledged` | `attempt_id`, `provider_request_id` when known |
| `provider_event`, `tool_event` | `attempt_id`, `event_type`, `content` |
| Control events | `control_id`, `actor`, plus instruction for `session_redirected` |
| `outcome_unknown` | `attempt_id`, `reason`, `reconciliation_options[]` |
| `outcome_reconciled` | `attempt_id`, `resolution`, `replacement_attempt_id` for retry |
| Terminal events | `attempt_id` when applicable, `summary`, `correlation_id` |
| `session_exported` | `export_id`, `format`, `recipient_fingerprints[]` |
| `session_deletion_requested` | `deletion_id`, `actor` |
| `session_deleted` | `deletion_id`, `key_destroyed`; emitted from the non-sensitive deletion tombstone, not appended to the encrypted event log |

Credential values, platform-key material, raw provider sessions, and unredacted
tool secrets are forbidden in every wire and persisted event.

Repeating an approval with the recorded decision returns its original result.
A second, conflicting decision for that approval returns
`invalid_transition` and changes no state.

## Attempts, Retries, and Recovery

An external effect has one stable attempt ID and the ordered facts
`dispatch_planned`, `dispatch_started`, optional `dispatch_acknowledged`, then a
definite terminal event or `outcome_unknown`. Recovery treats every started
attempt without a definite terminal fact as unknown.

Only an `idempotent-read` operation whose capability contract also declares
`idempotent: true` may be retried automatically, and only when failure is
proven to have occurred before `dispatch_started`. Paid inference, every write
or mutation, production access, credential access, and all unclassified
operations never retry automatically. Human `retry` creates a new attempt
linked to the uncertain attempt; `accept_result` and `abandon` never invoke the
adapter. Abandonment reaches the distinct `abandoned` terminal state and never
claims that external cancellation succeeded.

## Backpressure and Stable Errors

A subscription closes with `client_lagged` when the next encoded event would
exceed 1,024 queued events or 8 MiB of encoded frames. The daemon makes one
non-blocking attempt to send that final control error outside the bounded event
queue, then disconnects only that client. The session continues and the client
may replay from its durable cursor.

`retryable` is `false` for validation, authorization, policy, state,
`outcome_unknown`, and key-store failures. Provider and storage availability
errors are retryable only when no external attempt reached
`dispatch_started`. `internal` is never automatically retryable. Every error
contains a fresh correlation ID and a redacted message.
