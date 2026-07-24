# Tasks: Real-Time VS Code Workflow Controls

## Task Breakdown

- [x] T001 Specialize event rendering for routing_planned, workflow_transition,
  approval, dispatch lifecycle, terminal outcomes, and simple diffs/artifacts
  in `extensions/workbench-vscode/src/render.ts` with offline tests.
- [x] T002 Extend the protocol client with `session.approval.resolve` (grant/
  deny) and offline fake-transport coverage in protocol tests.
- [x] T003 Wire VS Code approval command, status bar workflow summary, session
  document control header, and activation events for all control commands in
  `extension.ts` without adding orchestration logic.
- [x] T004 Add Feature 009 acceptance harness in `workbench-testkit` that
  fingerprints Gherkin scenarios and binds them to extension test evidence;
  prove thin-client constraints remain.
- [x] T005 Accept ADR 0010; update README capability line, `llms.txt` if needed,
  and `docs/project/STATUS.md` for Feature 009 / issue #15.
- [ ] T006 Run offline extension tests, Feature 009 cargo harness, and
  `speckit validate`; open PR `refs #15` after green CI (merge when green per
  operator instruction for this issue).

## Dependencies

- Features 001–008 on `main` (especially 002 bridge and 008 workflow events).
- Issue [#15](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/15).
- Spec artifacts: `spec.md`, `plan.md`, ADR 0010, CUE schema, Gherkin feature.
