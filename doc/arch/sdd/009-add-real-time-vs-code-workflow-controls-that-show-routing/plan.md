# Implementation Plan: Real-Time VS Code Workflow Controls

## Overview

Extend `extensions/workbench-vscode` so an attached session becomes a real-time
workflow control room: routing plans, stages, progress, terminal outcomes, and
human controls (pause/resume/cancel/redirect/approval) over the existing local
protocol. No new daemon features beyond presentation of Feature 008 events.

## Technical Approach

### Layers

| Layer | Change |
|---|---|
| `extensions/workbench-vscode/src/protocol.ts` | Add `session.approval.resolve`; keep thin client. |
| `extensions/workbench-vscode/src/render.ts` | Specialize Markdown for routing, workflow, approval, dispatch, terminal, diffs/artifacts. |
| `extensions/workbench-vscode/src/extension.ts` | Status bar summary, approval command, control summary header, activation events. |
| Offline Node tests | Cover approval wire path and specialized rendering. |
| `workbench-testkit` | Feature 009 harness fingerprinting Gherkin → extension evidence. |
| Spec corpus / STATUS | ADR 0010, README/STATUS lines for delivered capability. |

### Control summary derivation

Maintain an in-memory `WorkflowControlSummary` updated from events:

- `workflow_transition` → workflow_id, run_id, step_id, iteration, phase, reason
- `approval_requested` → pending_approval_id
- `approval_recorded` / terminal outcomes → clear pending approval
- attach result / control replies may refresh session_state when present

Render the summary as a leading Markdown section and mirror a short form in the
status bar. Dedup and cursor logic remain Feature 002's SessionController.

### Non-goals

- No provider adapters, credentials, routing, or policy in TypeScript.
- No durable transcript files.
- No new protocol methods beyond existing `session.approval.resolve`.

## Companion Artifacts

- ADR 0010 (decision)
- CUE schema for control summary / approval command
- Gherkin feature under `doc/arch/specs/features/`
- Optional `quickstart.md` only if operator steps diverge from Feature 002

## Verification

1. Offline: `npm test` in `extensions/workbench-vscode` (compile + node:test).
2. `cargo test -p workbench-testkit --test feature_009 --locked` (after harness exists).
3. `speckit validate` green.
4. CI vscode extension job green on PR.
