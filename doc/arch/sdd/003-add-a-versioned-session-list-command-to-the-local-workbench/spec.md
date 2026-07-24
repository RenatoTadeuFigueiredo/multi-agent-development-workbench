---
id: 019f942e-ee7a-7e93-b658-000735390150
number: 003
slug: add-a-versioned-session-list-command-to-the-local-workbench
status: implemented
created_at: 2026-07-24T12:52:11.514814Z
---
# Feature Specification: Add a Versioned Session List Command to the Local Workbench

Feature: 003-add-a-versioned-session-list-command-to-the-local-workbench
Created: 2026-07-24

## User Stories

- As a CLI user, I want to list the persistent sessions served by my local
  Workbench daemon so that I can select one without copying an identifier from
  another tool.
- As a VS Code user, I want to create or select a session for the workspace's
  configured daemon so that I can continue work in the correct local context.
- As a presentation-client maintainer, I want a bounded metadata-only list
  operation so that discovery never transfers prompts, events, configuration,
  or other session content.

## Functional Requirements

1. `workbench/1` shall expose `session.list` as a daemon-scoped, read-only
   command with no `session_id` envelope field.
2. `session.list` shall accept `limit` (default `50`, inclusive range `1..=100`)
   and an optional exclusive `before_session_id` cursor. It shall return a
   deterministic, bounded page and an optional next-page cursor.
3. Each result item shall contain only session discovery metadata: `session_id`,
   folded lifecycle `state`, `created_at`, and optional `terminal_at`. It shall
   not include prompts, events, configuration, lock hashes, provider output,
   credentials, audit details, or encrypted storage material.
4. The CLI shall expose `workbench session list --limit <n>
   [--before-session-id <uuid>]` and map it directly to `session.list`.
5. The VS Code extension shall retain attach-by-ID and add **New Session** and
   **Select Session** commands. New Session shall send `session.create` with
   `persistent: true` and attach the resulting session; Select Session shall
   call `session.list` and present the returned metadata in a Quick Pick before
   attaching the chosen session.
6. Session discovery is local to the resolved daemon endpoint. The active VS
   Code workspace determines that endpoint through `workbench.endpoint`; the
   extension shall not enumerate sessions across endpoints or workspaces.
7. Clients shall negotiate `workbench/1` before listing, show bounded redacted
   errors for unavailable or incompatible daemons, and leave no discovery data
   persisted by default.
8. Clients shall reject a `session.list` result with more than 100 summaries,
   unknown result or summary fields, non-UUIDv7 identifiers, an unknown
   lifecycle state, or malformed RFC 3339 timestamps.
9. Default state, lock, and endpoint paths shall be isolated by a stable
   identifier derived from the canonical workspace path. If the legacy global
   database exists and the selected workspace database does not, startup shall
   fail with an actionable migration error rather than copying, assigning, or
   ignoring the legacy database.

## Security Requirements

The list operation exposes limited local-session metadata, so it must preserve
the existing same-user daemon boundary and avoid turning discovery into a
transcript API.

- **Data sensitivity/classification.** The response exposes only opaque UUIDs,
  lifecycle state, creation time, and terminal time when present. Prompts,
  events, configuration, hashes, provider data, and keys remain absent.
- **Authentication/authorization.** No new credential or authorization surface
  is introduced. The existing owner-only local socket and peer verification
  protect `session.list`.
- **Input validation.** `limit` is constrained to `1..=100` and
  `before_session_id` is parsed as UUIDv7. Unknown parameters and malformed
  frames are rejected by the versioned protocol codec. Presentation clients
  also enforce the bounded metadata-only result shape before retaining picker
  items.
- **Cryptography in transit/at rest.** Persistent session data remains in the
  existing encrypted store. Listing reads only indexed metadata and adds no
  plaintext cache or new transport. A legacy database is never copied to a new
  canonical path because its key namespace is bound to the original location.
- **Logging/audit.** Diagnostics retain request and correlation identifiers and
  stable error categories only. They do not log session content or a complete
  result page.
- **Error-handling information exposure.** CLI and VS Code errors are
  user-actionable and redact socket paths, storage details, and response bodies.

## Acceptance Scenarios

- Given a local daemon with persistent sessions
  When a compatible client sends `session.list` with a valid limit
  Then it receives a bounded page of metadata-only summaries in deterministic
  order.
- Given a page cursor equal to a returned session ID
  When the client requests the next page with `before_session_id`
  Then the cursor session is excluded and no earlier summary is repeated.
- Given a CLI user
  When they run `workbench session list --limit 20`
  Then the CLI sends `session.list` without a `session_id` envelope field.
- Given a VS Code workspace configured for one local endpoint
  When the user chooses **Select Session**
  Then the extension lists only that endpoint's sessions, shows their metadata
  in a Quick Pick, and attaches the selected session.
- Given a VS Code user
  When they choose **New Session**
  Then the extension creates a persistent session and attaches it.
- Given an unavailable or incompatible daemon
  When a client attempts discovery
  Then it displays a redacted actionable error and does not retain session data.
- Given a legacy global database and no database for the selected workspace
  When the workspace-scoped daemon resolves its runtime paths
  Then startup fails closed and directs the operator to the explicit migration
  runbook without creating or copying a database.

## Observability

The daemon records command latency and stable outcome categories for
`session.list`, correlated by request ID without session-content labels. The
VS Code bridge displays bounded in-memory connection errors; it emits no
transcript or result-page telemetry. Existing application-boundary OTLP
conventions continue to apply.

## Clarifications

- `before_session_id` is an exclusive cursor, not a filter for one session.
- Discovery is scoped to exactly one resolved local endpoint; there is no
  global or multi-workspace session registry.
- The legacy global database contains no trustworthy workspace identity, so it
  cannot be assigned automatically. This release supports encrypted export
  from the previous release but does not yet provide `session import`;
  continued legacy access and fresh workspace startup are therefore explicit
  operator choices documented in `doc/arch/operations/operations.md`.
