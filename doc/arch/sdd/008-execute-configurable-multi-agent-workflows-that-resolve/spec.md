---
id: 019f9625-70a7-7ba1-9d05-09d572e483c0
number: 008
slug: execute-configurable-multi-agent-workflows-that-resolve
status: analyzed
created_at: 2026-07-24T22:01:03.911956Z
---
# Feature Specification: Configurable Multi-Agent Workflow Executor

Feature: 008-execute-configurable-multi-agent-workflows-that-resolve
Created: 2026-07-24
Related issue: #14

## Objective

Give the Workbench daemon a versioned workflow executor that runs declarative
multi-stage workflows over provider-neutral roles and model aliases. The
executor must advance sequential stages and bounded review-correction loops,
persist every transition, explain every dispatch, honor pause/resume/cancel/
redirect, recover after interruption, and prove the primary Claude → Codex →
Grok → Codex path with offline fakes only.

## Scope

This feature includes:

- versioned declarative workflow validation beyond the Feature 001 shape
  (ordered steps, role binding, optional `on_findings` correction targets,
  `max_iterations`, optional per-step tool allowlists, optional fallbacks);
- a daemon-owned workflow runtime that selects the active step, resolves the
  step role to a model alias and provider adapter, and dispatches through the
  existing orchestrator attempt contract;
- sequential stage advancement and bounded correction loops when a step emits
  findings that route to `on_findings`;
- human controls already on the session lifecycle: pause, resume, cancel, and
  redirect applied to the active workflow run;
- durable workflow run state (workflow id, step id, iteration, phase,
  configuration snapshot hash) and explainable routing plans for each stage;
- deterministic recovery after daemon restart or provider interruption using
  persisted events and attempt progress;
- offline acceptance harness proving the primary multi-provider path with
  fake Claude, Codex, and Grok adapters.

This feature excludes:

- real-time VS Code workflow control room UX (issue #15);
- OpenRouter provider and cost controls (issue #16);
- Workbench ACP server and terminal client (issue #17);
- free-form DAG or parallel fan-out scheduling beyond sequential steps with
  optional single correction edges;
- automatic unpaid live provider quota in default tests;
- expanding the central MCP gateway surface (Feature 007 remains the tool
  authority; this feature only consumes it when a step uses allowlisted tools);
- editing Git history, rebasing, or external CI orchestration.

## User Stories

- As a workflow author, I want declarative multi-stage workflows that bind each
  step to a role and model alias so I can change providers without rewriting
  the process.
- As a developer, I want the primary Claude → Codex → Grok → Codex path to run
  end-to-end with durable history and a clear routing plan before each
  dispatch.
- As a developer, I want review findings to open a bounded correction loop that
  cannot run forever.
- As a local operator, I want pause, resume, cancel, and redirect to remain
  durable and visible to every attached client.
- As a tester, I want default suites to prove the executor with offline fakes
  only, without network or paid quota.

## Functional Requirements

1. **FR-008-001:** Resolved configuration MUST accept versioned workflows whose
   steps declare a stable `id`, a `role` that exists in `roles`, optional
   `on_findings` target step id, optional `max_iterations` (default 1, maximum
   8), optional step tool allowlist, and optional ordered fallback model
   aliases. Invalid graphs (missing roles, unknown `on_findings`, cycles that
   are not self-bounded by `max_iterations`, empty step lists) MUST fail
   configuration validation before lock generation.
2. **FR-008-002:** Starting a workflow run MUST pin the resolved configuration
   snapshot hash already required by Feature 001 sessions, record a durable
   `workflow_run` identity, and set the active step to the first declared step.
3. **FR-008-003:** For each active step the executor MUST produce a routing plan
   that names workflow id, step id, iteration, role, model alias, provider,
   permission scope, and the rule `workflow`. Dispatch MUST go through the
   existing orchestrator attempt path so one input never fans out to multiple
   executors unless distinct sequential stages request them.
4. **FR-008-004:** After a successful terminal step attempt without findings
   that require correction, the executor MUST advance to the next sequential
   step. After the last step succeeds, the workflow run MUST complete.
5. **FR-008-005:** When a step result is classified as having findings and the
   step declares `on_findings`, the executor MUST transition to that target
   step and increment the correction iteration counter. When `max_iterations`
   is exhausted without a clean pass, the run MUST pause for human decision
   with an explainable reason and MUST NOT auto-loop further.
6. **FR-008-006:** Fallback model aliases declared on a step MAY be selected
   only when preflight reports the primary provider unavailable or capability-
   incompatible. Fallbacks MUST be visible in the routing plan and MUST NOT
   silently change role identity.
7. **FR-008-007:** Session controls `pause`, `resume`, `cancel`, and `redirect`
   MUST apply to the active workflow run: pause freezes advancement; resume
   continues from the durable active step; cancel terminates the run and
   confirms or leaves outcome_unknown per attempt rules; redirect injects
   additional instruction for the next dispatch without rewriting history.
8. **FR-008-008:** After daemon restart the executor MUST recover run phase and
   active step from durable events. An in-flight attempt that never reached a
   definite terminal MUST remain `outcome_unknown` until human reconciliation;
   the executor MUST NOT invent success.
9. **FR-008-009:** Tool calls from workflow steps MUST continue to honor
   Feature 007 gateway policy (role ∩ workflow-step allowlists, approvals,
   pins). The executor MUST NOT bypass the gateway.
10. **FR-008-010:** Default automated tests MUST prove the primary path
    Claude-role → Codex-role → Grok-role → Codex-role using offline fakes only,
    with zero network and zero paid quota. Live providers remain opt-in.
11. **FR-008-011:** Public protocol and audit surfaces for workflow transitions
    MUST carry bounded identifiers (workflow id, step id, iteration, phase,
    correlation ids) and MUST NOT include raw prompts, secrets, or tool
    payloads.

## Security Requirements

- **Data sensitivity/classification.** Workflow runs carry user prompts, model
  outputs, and tool correlation identifiers already classified as session
  content under Feature 001 encryption. No new secret material is introduced.
- **Authentication/authorization.** No new credential surface. Provider auth
  remains behind existing adapters; tools remain behind the Feature 007
  gateway and approval protocol.
- **Input validation.** Workflow definitions are validated at configuration
  load. Runtime step transitions reject unknown step ids and over-limit
  iterations. Protocol controls reuse existing validation.
- **Cryptography in transit/at rest.** Session events and configuration
  snapshots continue to use Feature 001 encrypted SQLite and platform key
  stores. No cleartext durable workflow payload stores.
- **Logging/audit.** Workflow phase transitions emit bounded redacted events.
  Prompt bodies and tool payloads remain off by default.
- **Error-handling information exposure.** Failures use closed categories
  (`invalid_configuration`, `policy_denied`, `approval_required`,
  `outcome_unknown`, `provider_unavailable`) without paths, tokens, or raw
  provider frames.

## Acceptance Scenarios

1. **Validate workflows:** Given a configuration with a well-formed multi-step
   workflow and another with a missing role reference, when configuration is
   validated, then the well-formed workflow is accepted and the broken one
   fails closed.
2. **Sequential advancement:** Given a three-step workflow and offline fakes,
   when the run starts, then each step dispatches in order with an explainable
   routing plan and the run completes after the last success.
3. **Primary multi-provider path:** Given roles bound to Claude, Codex, Grok,
   and Codex again, when the primary workflow runs offline, then four attempts
   complete through the matching fake adapters in that order.
4. **Bounded correction loop:** Given a review step with `on_findings` and
   `max_iterations=2` that keeps reporting findings, when the loop exhausts,
   then the run pauses for human decision and does not dispatch a third
   automatic correction.
5. **Fallback routing:** Given a step whose primary provider is unavailable and
   a compatible fallback alias, when the step is entered, then the routing plan
   selects the fallback and records the reason.
6. **Pause and resume:** Given an active workflow run, when the user pauses and
   later resumes, then no dispatch occurs while paused and the next step
   continues from the durable active step.
7. **Cancel:** Given an active step attempt, when the user cancels, then the
   attempt follows existing cancel semantics and the workflow run terminates
   without silent success.
8. **Redirect:** Given a paused or active run, when the user redirects with new
   instruction, then history is not rewritten and the next dispatch includes
   the redirect instruction.
9. **Recovery after interruption:** Given a durable run mid-workflow and a
   daemon restart, when the session is recovered, then the active step and
   phase match the last durable facts and uncertain attempts stay unknown.
10. **Gateway tools remain governed:** Given a step tool allowlist, when the
    step requests a tool outside the allowlist, then Feature 007 denies before
    transport.
11. **Default suite offline:** Given default test configuration, when Feature
    008 acceptance runs, then no network sockets or live provider quota are
    used.

## Observability

- Metrics: workflow runs started/completed/paused/cancelled, step transitions,
  correction iterations, fallback selections, recovery loads (bounded labels:
  phase, outcome class).
- Logs: structured workflow id, step id, iteration, phase, selected rule;
  never prompts or secrets.
- Traces: correlation id from session input through step dispatch and attempt
  lifecycle.

Conventions live in `doc/arch/observability/observability.md`.

## Out of scope notes

- VS Code control room, OpenRouter, Workbench ACP server, and free-form DAG
  execution remain later increments.
- Feature 007 owns MCP lifecycle; this feature only sequences roles/stages.

## Clarifications
