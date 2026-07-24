# Tasks: Configurable Multi-Agent Workflow Executor

## Task Breakdown

- [ ] T001 Extend workflow configuration validation (and optional `fallbacks`
  field) in `workbench-config` + schema/fixtures so missing roles, bad
  `on_findings`, empty steps, and illegal `max_iterations` fail closed.
- [ ] T002 Add domain `WorkflowRun` state machine in `workbench-core` with
  pure transitions for start, advance, findings/correction, pause, resume,
  cancel, complete, and fail; unit-test bounds and illegal transitions.
- [ ] T003 Emit durable workflow transition facts (bounded ids/phase/iteration)
  through the session event path without storing raw prompts or tool payloads.
- [ ] T004 Compose workflow start and step dispatch in `workbench-daemon`
  using existing orchestrator attempts, routing plans (`SelectedRule::Workflow`),
  and provider registry resolution from step roles.
- [ ] T005 Implement sequential advancement and bounded `on_findings` loops
  with `max_iterations` ceiling and `awaiting_human` when exhausted.
- [ ] T006 Wire pause, resume, cancel, and redirect controls to the active
  workflow run with durable phase updates.
- [ ] T007 Implement recovery: rebuild active run from events after restart;
  preserve `outcome_unknown` for interrupted attempts.
- [ ] T008 Integrate step tool allowlists with Feature 007 gateway policy
  (deny-before-transport proof in acceptance).
- [ ] T009 Implement Feature 008 offline acceptance harness in
  `workbench-testkit`: fingerprint Gherkin cases; prove Claude→Codex→Grok→Codex
  path, correction bound, controls, recovery, offline defaults.
- [ ] T010 Update ADR 0009 to accepted, operations notes if needed, README
  capability line, and `docs/project/STATUS.md`.
- [ ] T011 Run `speckit validate`, `make check`, and Feature 008 acceptance;
  present reviewed summary for human approval before PR (`refs #14`).

## Dependencies

- Features 001–007 delivered on `main`.
- Issue [#14](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/14).
- Spec artifacts: `spec.md`, `plan.md`, ADR 0009, feature CUE, Gherkin feature.
