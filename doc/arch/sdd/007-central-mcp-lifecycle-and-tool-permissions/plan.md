# Implementation Plan: Central MCP Lifecycle and Tool Permissions

## Overview

Implement a daemon-owned MCP and tool gateway that activates Feature 001's
registry, effect-class, policy, and approval contracts for live supervised
stdio MCP children and remote HTTP MCP clients. The gateway is the single
pinning, allowlist, approval, audit, and lifecycle authority for shared tools
across Grok, Claude, Codex, and future API providers. Feature 007 does not
build the multi-agent workflow executor, OpenRouter, VS Code control room, or
provider-native write bridges.

## Technical Approach

### Configuration and lock (extend existing crates)

Expand `workbench-config` MCP server models beyond transport/version/sha256 to
the minimal launch/connect identity required by the gateway:

- stdio: absolute user-owned executable or package entrypoint path, optional
  argv suffix, optional env secret-handle map (opaque `platform:` refs only);
- http: absolute URL, optional TLS pin metadata, optional header secret-handle
  map.

Keep `tools` of kind `mcp` requiring `mcp_server` plus operations with
`effect_class`, `idempotent`, `material_cost`, and `approval`. Strengthen
semantic validation for cross-references, illegal idempotent combinations, and
lock pin coverage. Lock generation pins every configured MCP id with version
and sha256; session locks still cannot introduce an MCP absent from the base
lock.

Schema updates land in
`doc/arch/datamodels/workbench-configuration.schema.json` and
`workbench-lock.schema.json` with mirrored generated fixtures under
`workbench-testkit`.

### Gateway crate and daemon composition

Add a focused crate (preferred name `workbench-mcp`) that depends on
`workbench-core` and configuration types, not on provider adapters or UI:

- registry load and pin verification;
- stdio process supervision (direct argv, no shell, bounded stderr, workspace
  isolation, reap/shutdown);
- HTTP client with TLS for non-loopback, unpinned-redirect rejection, and 8 MiB
  default response ceiling;
- tool invoke path that returns only redacted lifecycle outcomes;
- cancellation and shutdown coordination.

`workbench-daemon` composes the gateway at startup after configuration and lock
validation, exposes it to the orchestrator/policy path for tool attempts, and
includes it in graceful shutdown. Domain policy remains in `workbench-core`
(`resolve_tool_policy`, `protect_effect`, approvals). The gateway enforces;
it does not reimplement monotonic intersection.

### Policy and allowlists

Wire role tool lists and optional workflow-step restrictions into the existing
policy layer stack so effective grants are:

`built-in ∩ user ∩ repository ∩ session ∩ role ∩ workflow ∩ effect-class`.

Unlisted tools default to denied. Repository grants cannot widen user denies.
Approval uses existing `approval_requested` / `session.approval.resolve`
before any external MCP call for mandatory effect classes and
`approval: always` / policy-required operations.

### Attempt and audit semantics

Reuse attempt identity and ordered facts: planned (effect class + operation),
started at external call begin, definite terminal or `outcome_unknown`. Tool
events on the public protocol carry only bounded tool name, lifecycle
category, redacted outcome, and correlation ids. No raw arguments, results,
frames, paths, or secrets enter SQLite, WAL, exports, logs, or IPC.

Automatic retry only for proven pre-start failures of idempotent reads that
also declare `idempotent: true`.

### Provider boundary

Features 004–006 continue to disable provider-local MCP registration. Shared
capabilities are offered only as Workbench-managed MCP tools. No Claude/Codex/
Grok native write or shell enablement in this feature.

### Testing Strategy

- Unit tests in `workbench-mcp` for pin checks, path safety, HTTP bounds,
  allowlist denials, approval gating, frame/response ceilings, and redaction.
- Committed offline fakes: `fake_mcp_stdio` and `fake_mcp_http` (or one binary
  with modes) under `workbench-testkit`.
- Feature 007 acceptance harness fingerprinting every concrete Gherkin case in
  `central-mcp-lifecycle-and-tool-permissions.feature`.
- Daemon composition tests: empty registry, pin mismatch fail-closed, dual-
  workspace stdio isolation, shutdown reaping, coexistence with provider
  adapters.
- Default `make check` remains network-free and quota-free. Any live MCP check
  is `#[ignore]` and opt-in.

### Risks

- MCP protocol dialects vary; freeze a narrow invoke/list contract and fail
  closed on unknown authority-granting methods.
- Stdio children are same-user processes, not OS sandboxes; path and
  permission checks reduce risk but do not replace least-privilege tools.
- HTTP MCP endpoint drift and redirects can create confused-deputy paths;
  pin endpoint identity and reject unpinned redirects.
- Redaction reduces operator visibility into raw tool I/O; compensate with
  stable error categories and optional operator-side server logs outside
  Workbench storage.
- Expanding configuration schema is a contract change; keep backward
  compatibility for empty MCP maps and existing provider configs.

## Companion Artifacts

- ADR 0008 records the daemon-owned gateway decision and rejected alternatives.
- Feature CUE captures lock, runtime, policy, and testing constraints.
- Gherkin feature supplies offline acceptance scenarios for pins, allowlists,
  approvals, isolation, bounds, cancellation, secrecy, empty registry, and
  shutdown.
- Archived draft `099-central-mcp-lifecycle-and-tool-permissions` is inert
  Speckit history from a specify collision; implement only against Feature 007.
- Optional later companions: `research.md` (MCP transport choices),
  `quickstart.md` (operator pin and lock steps). Threat-model and operations
  updates land during implement when runtime behavior is concrete.
- Persistent data remains the existing encrypted session event model; gateway
  process handles are runtime-only.
