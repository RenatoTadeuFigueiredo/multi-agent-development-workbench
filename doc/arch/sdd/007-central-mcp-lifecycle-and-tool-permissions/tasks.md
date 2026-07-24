# Tasks: Central MCP Lifecycle and Tool Permissions

## Task Breakdown

- [x] T001 Extend configuration and lock contracts for MCP launch identity:
  update `workbench-configuration.schema.json`, `workbench-lock.schema.json`,
  `workbench-config` models/validation, and generated testkit fixtures so each
  `mcp_servers` entry can carry transport-specific endpoint data and opaque
  secret handles while retaining empty-registry validity and base-lock pin
  rules (version + sha256).
- [x] T002 Scaffold `workbench-mcp` in the workspace with `forbid(unsafe_code)`,
  modules for registry, stdio supervision, HTTP client, invoke/redaction,
  error categories, and offline unit tests that compile without network.
- [x] T003 Implement pin verification and fail-closed availability: compare
  configured artifacts to lock digests/versions; reject symlinks, unsafe path
  components, group/world-writable executables, missing artifacts, and
  unsupported transports before any tool call.
- [x] T004 Implement supervised stdio MCP lifecycle: direct argv launch without
  a shell, private runtime working directory, piped stdio, bounded stderr
  discard, workspace isolation (no shared child across workspaces), cancel,
  kill escalation, drain, and proven reaping on shutdown.
- [x] T005 Implement HTTP MCP client: pinned endpoint identity, TLS for
  non-loopback hosts, reject redirects to unpinned hosts, default 8 MiB
  encoded response ceiling, and stable redacted transport errors.
- [x] T006 Wire role and workflow allowlists into policy resolution so
  effective grants are built-in ∩ user ∩ repository ∩ session ∩ role ∩
  workflow ∩ effect-class; unlisted tools denied; repository cannot widen
  user-global denies.
- [x] T007 Integrate approval gating with the existing protocol: for mandatory
  effect classes and approval-required operations emit `approval_requested`,
  wait for `session.approval.resolve`, start the MCP call only on `grant`, and
  record durable denials without transport on `deny`.
- [x] T008 Implement gateway invoke path and attempt semantics: stable attempt
  ids; planned/started/terminal or `outcome_unknown`; automatic retry only for
  proven pre-start failures of idempotent reads; public tool events limited to
  bounded name, lifecycle category, redacted outcome, and correlation ids.
- [x] T009 Compose the gateway in `workbench-daemon` startup, health, and
  shutdown without provider conditionals in the domain; keep Features 004–006
  provider-local MCP registration disabled; expose shared tools only through
  the gateway.
- [x] T010 Add committed offline fakes (`fake_mcp_stdio` / `fake_mcp_http` or
  equivalent modes) covering success, deny, pin mismatch, oversized response,
  unpinned redirect, hang/cancel, crash after start, and empty-registry paths
  with no network or credential store access.
- [x] T011 Implement Feature 007 acceptance harness in `workbench-testkit`:
  fingerprint every concrete Gherkin case in
  `central-mcp-lifecycle-and-tool-permissions.feature`, bind each to
  gateway/daemon/fake evidence, and prove offline zero-quota zero-network
  defaults.
- [x] T012 Prove secret containment: unique markers in arguments, results,
  environment, and credential handles absent from replies, telemetry, locks,
  SQLite, WAL, exports, and logs on success and failure paths.
- [x] T013 Update threat model, operations/runbook notes, README capability
  lines, and `docs/project/STATUS.md` for delivered MCP gateway behavior;
  keep workflow executor, OpenRouter, VS Code control room, and Workbench ACP
  server explicitly out of scope.
- [ ] T014 Run `speckit validate`, `make check`, and Feature 007 acceptance;
  fix findings; present reviewed summary for human approval before opening a
  pull request (`refs #13`).

## Dependencies

- Features 001–006 on `main` (provider ports, policy, approvals, lock, daemon
  composition).
- Issue [#13](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/13).
- Spec artifacts: `spec.md`, `plan.md`, ADR 0008, feature CUE, Gherkin feature.
- No dependency on issues #14–#17 for acceptance of this feature.
