---
id: 019f9665-06e5-7ad2-8a42-b75e3d0c2b0b
number: 009
slug: add-real-time-vs-code-workflow-controls-that-show-routing
status: implemented
created_at: 2026-07-24T23:10:31.142069Z
---
# Feature Specification: Real-Time VS Code Workflow Controls

Feature: 009-add-real-time-vs-code-workflow-controls-that-show-routing
Created: 2026-07-24
Related issue: #15

## Objective

Turn the existing thin VS Code session bridge into a real-time control room for
durable multi-agent workflows. Users attach once, see routing plans, provider
stages, progress, and terminal outcomes, and can pause, resume, cancel,
redirect, and resolve approvals—without embedding orchestration, providers,
credentials, or policy in TypeScript.

## Scope

This feature includes:

- rendering workflow-aware session documents from versioned local protocol
  events (`routing_planned`, dispatch lifecycle, `workflow_transition`,
  approval, provider/tool events, and terminal outcomes);
- status-bar and document summary surfaces for session state, active workflow
  step, iteration, and phase;
- VS Code commands for prompt, pause, resume, cancel, redirect, and approval
  grant/deny over the existing NDJSON protocol;
- reattach after editor restart or transport loss using the durable event
  cursor without duplicate rendering;
- offline deterministic extension tests and a Feature 009 acceptance binding
  that fingerprints Gherkin without network or paid quota.

This feature excludes:

- OpenRouter provider and cost controls (issue #16);
- Workbench ACP server and terminal client (issue #17);
- new daemon orchestration, routing, or policy logic in TypeScript;
- provider SDKs, credential stores, or remote network access from the
  extension;
- free-form workflow authoring UI or DAG editors;
- changing the Feature 008 executor semantics.

## User Stories

- As a developer, I want to watch a multi-agent workflow from VS Code so I can
  see which role, model, and step is active without leaving the editor.
- As a developer, I want pause, resume, cancel, redirect, and approval controls
  in the same surface so I can intervene when the daemon awaits a human.
- As a developer, I want reattachment after restart so I resume the same
  durable session without duplicate events.
- As a maintainer, I want the extension to remain a thin protocol client so the
  editor surface can be replaced without changing the kernel.

## Functional Requirements

1. **FR-009-001:** The extension MUST remain a versioned `workbench/1` local
   protocol client only. It MUST NOT implement routing, provider adapters,
   credential access, tool policy, or persistence.
2. **FR-009-002:** While attached, the extension MUST render a live session
   document that includes a workflow control summary (session id, session
   state when known, latest workflow id/step/iteration/phase from
   `workflow_transition` events) and chronological event sections.
3. **FR-009-003:** Events of kind `routing_planned` MUST surface destination
   role, model alias, provider, selected rule, risk, and permission when those
   fields are present in event data; missing fields MUST not crash rendering.
4. **FR-009-004:** Events of kind `workflow_transition` MUST surface workflow
   id, run id, step id, iteration, phase, and reason as bounded identifiers
   only (no raw prompts or tool payloads).
5. **FR-009-005:** Events of kind `approval_requested` MUST surface approval
   id, action, and risk; the user MUST be able to send
   `session.approval.resolve` with decision `grant` or `deny` for a selected
   pending approval id through a dedicated command.
6. **FR-009-006:** Existing controls `session.prompt`, `session.pause`,
   `session.resume`, `session.cancel`, and `session.redirect` MUST remain
   available as VS Code commands and MUST require an attached session.
7. **FR-009-007:** Provider Markdown content, fenced Mermaid, simple
   artifact/diff text carried in event data, and terminal outcomes
   (`session_completed`, `session_failed`, `session_cancelled`,
   `outcome_unknown`) MUST render through VS Code Markdown preview surfaces
   without writing a durable transcript by default.
8. **FR-009-008:** Reattach and reconnect MUST use the last observed sequence
   cursor and stable event ids to suppress duplicates after editor restart or
   transport loss.
9. **FR-009-009:** A status bar item MUST show the attached session id (short
   form) and the latest known workflow phase or session control state when
   available.
10. **FR-009-010:** Default automated tests MUST run offline with a fake
    protocol server/transport only—no daemon network, no provider credentials,
    no paid quota. Feature 009 Gherkin scenarios MUST be fingerprinted and
    bound to executable evidence.

## Security Requirements

- **Data sensitivity/classification.** Session prompts, responses, approvals,
  and workflow metadata are potentially sensitive. The extension holds only the
  active in-memory view; durable storage remains in the daemon. No new on-disk
  transcript is introduced by default.
- **Authentication/authorization.** No new credential surface. Owner-only local
  socket authorization and existing protocol checks remain the boundary.
  Approval decisions are recorded by the daemon with actor attribution.
- **Input validation.** Protocol frames, UUIDs, event sizes, and command params
  remain validated by the daemon codec. The extension validates client-side
  shapes needed for safe rendering and rejects invalid discovery results as in
  Features 002/003. Approval ids entered by the user are sent as opaque UUID
  strings without local policy evaluation.
- **Cryptography in transit/at rest.** Local transport and encrypted daemon
  storage retain existing protections; the extension adds no plaintext
  persistence.
- **Logging/audit.** User-facing notices are content-free connection and
  control acknowledgements. Prompt bodies, secrets, and tool payloads MUST NOT
  be written to extension logs.
- **Error-handling information exposure.** Errors use stable categories and
  omit socket paths, credentials, and response bodies.

## Acceptance Scenarios

- Given an attached workflow session that emits `routing_planned` and
  `workflow_transition` events
  When the VS Code bridge renders the session document
  Then routing destination fields and workflow step/iteration/phase appear in
  the Markdown preview without raw prompt text.

- Given an attached session in `awaiting_approval` with an `approval_requested`
  event
  When the user runs grant or deny
  Then the extension sends `session.approval.resolve` with the approval id and
  decision and does not evaluate policy locally.

- Given an attached running workflow
  When the user pauses, resumes, cancels, or redirects
  Then the corresponding versioned control command is sent and durable control
  events continue to stream into the document.

- Given a lost transport or editor restart after events were observed
  When the bridge reattaches from the durable cursor
  Then previously rendered event ids are not duplicated.

- Given the offline fake protocol suite
  When Feature 009 extension tests run
  Then they complete without network, credentials, or paid quota.

## Observability

The daemon remains the source of operational telemetry. The extension presents
bounded, content-free notices (reconnect, attach, control failures) in the
session document and status bar. Request ids on protocol commands remain
correlatable with daemon traces. No new high-cardinality metric labels are
introduced from the TypeScript surface.

## Clarifications

None pending at specify time. Feature 002 remains the thin bridge foundation;
this feature only extends presentation and control wiring over the existing
protocol surface delivered through Feature 008.
