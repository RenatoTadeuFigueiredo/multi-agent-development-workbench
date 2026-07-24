---
status: accepted
date: 2026-07-24
deciders: [workbench-maintainers]
consulted: []
informed: []
---

# Real-Time VS Code Workflow Controls Stay a Thin Protocol Client

## Context and Problem Statement

Feature 002 shipped a thin VS Code bridge for attach, prompt, and basic
lifecycle controls. Feature 008 completed multi-agent workflow execution in the
daemon. Users still lack a control-room presentation that shows routing plans,
workflow stages, progress, terminal outcomes, and approvals in real time.

The risk is bloating TypeScript with orchestration, policy, or provider logic
that must remain in the Rust kernel.

## Decision Drivers

- Keep a single orchestration authority in the daemon.
- Reuse the versioned `workbench/1` protocol without new remote APIs.
- Present workflow-specific facts already emitted as durable events.
- Preserve offline, deterministic extension tests.
- Support reattach without duplicate events.

## Considered Options

- **A — Thin presentation client:** extend the existing extension to render
  workflow/routing/approval events and wire approval + lifecycle commands only.
- **B — Embedded orchestrator in the extension:** run workflow planning and
  control logic in TypeScript.
- **C — Separate web UI / remote control plane:** introduce a new HTTP or cloud
  surface for workflow controls.

## Decision Outcome

Chosen option: **A — Thin presentation client**, because Feature 008 already
persists the facts needed for observation and the protocol already exposes
pause, resume, cancel, redirect, and approval resolve. The extension remains
replaceable and free of provider credentials.

### Consequences

- Good: one durable truth in the daemon; editor can be swapped; offline tests stay cheap.
- Good: reuses Feature 002 transport, cursor, and dedupe machinery.
- Bad: richer UX (graph editors, multi-session dashboards) is deferred.
- Bad: presentation quality depends on event data shape from the daemon.

## Implementation Notes

- Specialize Markdown rendering for `routing_planned`, `workflow_transition`,
  `approval_requested`/`approval_recorded`, dispatch lifecycle, and terminal
  outcomes.
- Add `session.approval.resolve` to the protocol client.
- Show a status bar summary of session/workflow phase.
- Bind Gherkin scenarios to offline Node tests and a Feature 009 acceptance
  harness fingerprint.
