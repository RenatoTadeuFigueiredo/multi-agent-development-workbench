# Implementation Plan: Configurable Multi-Agent Workflow Executor

## Overview

Implement a daemon-owned workflow runtime that executes Feature 001 declarative
workflows as durable multi-stage runs. The runtime advances sequential steps,
applies bounded `on_findings` correction loops, reuses the orchestrator attempt
contract and session controls, consumes Feature 007 tool policy when steps
invoke tools, and proves the primary Claude → Codex → Grok → Codex path with
offline fakes.

## Technical Approach

### Configuration validation

Extend `workbench-config` validation for workflows:

- non-empty ordered steps with unique ids;
- every `role` exists in `roles`;
- `on_findings` targets an existing step id when present;
- `max_iterations` defaults to 1 and rejects values outside 1..=8;
- optional `fallbacks` list of known model aliases;
- optional per-step `tools` remain a subset restriction of the role grant
  (already enforced at invoke time by Feature 007).

Schema updates land in `workbench-configuration.schema.json` only if new fields
are required; prefer reusing existing `Workflow` / `WorkflowStep` shapes and
add `fallbacks` if missing.

### Domain workflow runtime (`workbench-core`)

Add a focused workflow module (preferred names: `workflow` / `WorkflowRuntime`)
that is pure domain:

- `WorkflowRun` state: workflow id, run id, active step id, iteration, phase;
- closed phase set: planned, running, paused, awaiting_human, completed,
  cancelled, failed;
- pure transition functions for start, advance, findings, pause, resume,
  cancel, complete, fail;
- never invents success for uncertain attempts.

Routing continues to use `SelectedRule::Workflow` and existing
`RoutingPlan` / preflight / fallback selection from Feature 001.

### Daemon composition

`workbench-daemon` application layer:

- start workflow run on explicit protocol/CLI command (or session attach with
  workflow target — prefer explicit `workflow.start` if protocol extension is
  needed; otherwise session create with workflow id field already available);
- before each step dispatch, build `ExecutionRequest` with the step role's
  resolved adapter;
- after terminal outcomes, call domain transitions and append durable events;
- on recovery, rebuild `WorkflowRun` from event history.

Prefer extending session events with bounded workflow transition payloads
rather than a second store. Keep public event fields redacted and identifier-
only.

### Findings classification

For offline acceptance, findings are explicit structured signals from the fake
provider (for example a final event flag or normalized marker
`findings_present=true`). Natural-language heuristics are out of scope; live
adapters may map provider-specific review formats later behind the same port.

### Controls

Map existing session controls:

| Control | Workflow effect |
|---|---|
| pause | phase → paused; no advancement |
| resume | phase → running from durable step |
| cancel | cancel active attempt; phase → cancelled / unknown |
| redirect | record instruction; apply on next dispatch |

### Acceptance

Add `workbench-testkit/tests/feature_008.rs` that:

- fingerprints every concrete Gherkin case;
- runs sequential multi-provider path with existing fake Claude/Codex/ACP
  adapters;
- proves correction bound, pause/resume, cancel, redirect, recovery, and
  offline defaults;
- reuses Feature 007 gateway denial for step tool allowlists.

## File / crate impact

| Area | Change |
|---|---|
| `workbench-config` | workflow validation; optional `fallbacks` field |
| `workbench-core` | workflow run state + transitions; routing already has Workflow rule |
| `workbench-protocol` | optional workflow.start / transition events if not representable |
| `workbench-daemon` | compose runtime, recovery, dispatch loop |
| `workbench-testkit` | `feature_008` harness + fixtures |
| `doc/arch` | this plan, tasks, ADR 0009, Gherkin, CUE, STATUS |

## Dependencies

- Features 001–007 on `main` (merge SHA for 007: `40c666d`).
- Issue #14.
- No dependency on #15–#17 for acceptance.

## Risks

- Protocol expansion may require careful version negotiation; prefer reusing
  existing session commands when possible.
- Findings classification must stay deterministic for offline proof.
- Recovery of mid-attempt runs must not auto-complete.

## Rollback

Revert the feature branch / merge. Workflow configuration without an executor
already loads as inert data; removing the runtime restores that behavior.
