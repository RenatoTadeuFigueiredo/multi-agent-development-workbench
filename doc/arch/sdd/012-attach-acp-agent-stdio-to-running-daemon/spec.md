---
id: 019f993a-1a2b-7c3d-8e4f-0120a11ac001
number: 012
slug: attach-acp-agent-stdio-to-running-daemon
status: implement
created_at: 2026-07-25T12:35:00.000000Z
---
# Feature Specification: Attach ACP Agent Stdio to Running Daemon

Feature: 012-attach-acp-agent-stdio-to-running-daemon
Created: 2026-07-25
Related issue: #29

## Objective

Make production `workbench agent stdio` attach to a workspace-local running
daemon endpoint over the versioned NDJSON protocol so ACP clients share durable
sessions with the CLI and VS Code. Keep the in-process offline fake backend for
deterministic tests.

## Scope

Includes:

- `DaemonSocketBackend` (or equivalent) implementing `BridgeBackend` over the
  owner-only local Unix socket;
- CLI production path that discovers `RuntimePaths` and attaches fail-closed when
  the daemon is missing or unreachable;
- offline acceptance coverage for attach, prompt stream, cancel, session
  visibility to other local clients, and missing-daemon failure;
- retention of `InProcessBackend::offline_fake` for unit and Feature 011 paths.

Excludes:

- Grok-derived pager / WorkbenchBackend PTY (#33);
- OpenRouter live HTTPS / durable cost ledger (#31);
- provider-native write tools (#32);
- embedding Grok Build as the control plane.

## Functional Requirements

1. **FR-012-001:** Production `workbench agent stdio` MUST connect to the
   workspace-local daemon endpoint discovered via existing runtime path rules.
2. **FR-012-002:** ACP `initialize`, `session/new`, `session/prompt`, and
   `session/cancel` MUST complete against a live local daemon using the offline
   fake provider path in acceptance tests.
3. **FR-012-003:** Sessions created via ACP MUST appear to other local clients
   (for example `session.list`) for the same workspace daemon.
4. **FR-012-004:** Missing or unreachable daemon MUST fail closed with an
   actionable error and MUST NOT hang indefinitely.
5. **FR-012-005:** Default CI MUST remain offline and quota-free; the in-process
   backend MUST remain available for unit and Feature 011 acceptance.

## Success Criteria

- Offline acceptance harness for Feature 012 is green.
- STATUS Known Gaps removes or narrows the ACP attach bullet for #29.
- `speckit validate` green for the Feature 012 corpus.

## Observability

Socket attach failures report a stable backend error kind with a short, redacted
message (no host secrets). Session identifiers follow existing daemon redaction.
