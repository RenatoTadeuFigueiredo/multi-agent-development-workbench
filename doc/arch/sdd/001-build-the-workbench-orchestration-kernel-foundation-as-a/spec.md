---
id: 019f908d-d49d-71c1-8822-0fc4ad0f0069
number: 001
slug: build-the-workbench-orchestration-kernel-foundation-as-a
status: planned
created_at: 2026-07-23T19:57:21.949953Z
---
# Feature Specification: Orchestration Kernel Foundation

Feature: 001-build-the-workbench-orchestration-kernel-foundation-as-a
Created: 2026-07-23
Related issue: #1

## Objective

Establish the smallest provider-independent control plane that can resolve a
session configuration, route a prompt, execute it through a fake provider,
stream normalized events, and retain enough state for multiple clients to
observe and control the same work.

## Scope

This feature includes the Rust domain model, layered configuration resolution,
routing decisions, provider and client contracts, durable local session events,
policy decisions, and fake-adapter contract tests.

This feature excludes production provider authentication and model calls, the
VS Code extension, the Grok Build fork, a complete terminal interface, remote
or mobile access, live MCP server execution, and OpenRouter billing. Later
features must implement those capabilities against the contracts established
here.

## User Stories

- As a developer, I want one prompt to resolve to one visible role and provider
  so that I know where work is being performed.
- As a developer, I want VS Code and terminal clients to observe and control the
  same durable session so that switching interfaces does not lose context.
- As a workflow author, I want roles and model aliases resolved from layered
  configuration so that models can change without rewriting workflows.
- As an integration author, I want stable client and provider contracts so that
  new editors and providers do not add vendor branches to the core.
- As a security-conscious operator, I want least-privilege tools, explicit
  approvals, and redacted audit events so that agents cannot silently expand
  their authority.

## Functional Requirements

1. **FR-001:** A single local daemon MUST own orchestration state; clients MUST
   not make provider, routing, policy, or credential decisions.
2. **FR-002:** Configuration MUST resolve in this precedence order: safe
   built-ins, user configuration, repository configuration, then explicit
   session overrides. Invalid higher-precedence values MUST fail validation
   instead of silently falling back.
3. **FR-003:** Workflows MUST reference roles, roles MUST reference model
   aliases, and aliases MUST resolve to provider adapters and runtime model
   identifiers. Provider-specific behavior MUST remain behind adapter
   contracts.
4. **FR-004:** A new session MUST retain a redacted configuration snapshot,
   content hash, schema version, and resolved role/model/provider mapping.
   Later configuration changes MUST NOT alter an active session without an
   explicit validated migration.
5. **FR-005:** Every user input MUST be assigned a session sequence number and
   durably recorded before dispatch.
6. **FR-006:** Routing MUST apply, in order: explicit target or workflow
   command, active workflow step or attached session, deterministic resolver,
   configured coordinator, then user clarification. A lower-priority rule MUST
   NOT override a successful higher-priority rule.
7. **FR-007:** Before provider dispatch, the daemon MUST emit a routing plan
   containing intent, role, model alias, provider, context sources, tools,
   permission scope, risk, confidence, and the rule that selected the route.
8. **FR-008:** One input MUST resolve to at most one executor unless a workflow
   explicitly declares multiple stages. The daemon MUST NOT broadcast an input
   implicitly.
9. **FR-009:** Provider adapters MUST expose capability discovery,
   authentication status, session start and resume, prompt streaming,
   cancellation, tool events, completion, and normalized failures.
10. **FR-010:** Preflight MUST reject a route whose required capabilities are
    unavailable. It MAY select a configured compatible fallback; otherwise it
    MUST ask the user for direction without dispatching.
11. **FR-011:** Client contracts MUST negotiate a protocol version and expose
    ordered event replay from a cursor. An incompatible major version MUST fail
    with a stable compatibility error.
12. **FR-012:** Sessions MUST support `pause`, `resume`, `cancel`, and
    `redirect`. Controls MUST be idempotent, durably recorded, and visible to
    every attached client. A paused session MUST start no new provider or tool
    action.
13. **FR-013:** A cancelled session MUST request adapter cancellation, reject
    new work, preserve its event history, and reach a terminal state even when
    the adapter stops responding.
14. **FR-014:** Shared MCP tools MUST be addressed through a central registry
    and filtered by role and session policy. This feature models and validates
    grants but does not launch live MCP servers.
15. **FR-015:** Mutating tools, credential access, production access, and
    policy expansion MUST require the applicable approval. A repository
    configuration MUST NOT widen user-global permissions.
16. **FR-016:** Failures MUST include a stable category, retryability,
    user-safe message, and correlation identifier without exposing prompt
    content, credentials, or provider session material.
17. **FR-017:** Default automated tests MUST use deterministic fake clients,
    providers, clocks, and tools. The default test suite MUST make zero network
    calls and consume zero paid model quota.

## Clarified Decisions

- The daemon is local-only for this increment and accepts connections from the
  same user account.
- SQLite is the initial durable event store; the domain contract must not
  expose SQLite-specific types.
- Event sequence numbers are monotonic within a session. Reconnecting clients
  request replay after their last observed sequence.
- `pause` stops new orchestration actions but does not discard already received
  provider output. `redirect` appends new instruction and never rewrites
  history.
- Coordinator-assisted classification is represented by an adapter contract
  and tested with a fake coordinator; no real model is called.
- The protocol transport and serialization are plan decisions, while the
  versioning and behavioral guarantees in this specification are mandatory.

## Security Requirements

- **Data sensitivity/classification:** Prompts, repository paths, tool
  arguments, provider metadata, and artifacts can contain confidential source
  or personal data. Persistent snapshots and audit fields must be redacted by
  schema, not by best-effort string replacement.
- **Authentication/authorization:** The local daemon accepts only same-user
  clients. Provider credentials remain in provider-owned stores or the
  operating-system keychain and are referenced by opaque identifiers.
- **Input validation:** Configuration, protocol frames, model output, tool
  events, file paths, and cursor values are untrusted. Parsers must reject
  unknown schema majors, traversal, oversized frames, invalid transitions, and
  unresolved aliases before changing state.
- **Cryptography in transit/at rest:** Local transport must use operating-system
  access controls. This increment stores no credential values. Encryption of
  the local event database is deferred until a threat-model decision defines
  its key lifecycle.
- **Logging/audit:** Audit events record actor, action, target, decision, result,
  sequence, and correlation identifiers. They omit credentials and default to
  metadata rather than prompt or model-output bodies.
- **Error-handling information exposure:** User-facing failures expose stable
  categories and remediation while sensitive provider diagnostics remain
  redacted and locally restricted.

## Acceptance Scenarios

1. Given conflicting user, repository, and session values, when configuration
   is resolved, then the session value wins and the redacted snapshot records
   the resolved source and hash.
2. Given an explicit target in a prompt, when routing runs, then exactly one
   routing plan identifies that target and the coordinator is not invoked.
3. Given no deterministic route and a fake coordinator below the configured
   confidence threshold, when routing runs, then no provider receives the
   prompt and a clarification event is emitted.
4. Given a role requiring structured tool calls and a provider lacking that
   capability, when preflight runs, then a compatible configured fallback is
   selected or dispatch stops with a capability error.
5. Given two clients attached to one session, when either client pauses it,
   then both receive the same ordered pause event and no new adapter or tool
   action starts until resume.
6. Given a client disconnects after event 20, when it reconnects with cursor
   20, then it receives every later event once and in sequence order.
7. Given an unresponsive fake provider, when the user cancels the session, then
   the session reaches a terminal cancelled state and retains its prior events.
8. Given repository configuration grants a tool denied globally, when policy
   resolves, then the tool remains denied and the audit event identifies the
   higher-priority policy.
9. Given the default test command, when the complete suite runs, then network
   access and paid provider adapters are not invoked.

## Observability

The daemon emits structured events for configuration resolution, routing,
preflight, provider lifecycle, policy decisions, controls, replay, and terminal
outcomes. Metrics cover active sessions, routing results, denied actions,
adapter failures, cancellation latency, and replay lag with bounded labels.
Trace spans carry session and correlation identifiers, never prompt bodies or
credentials. OTLP export is disabled by default and follows
`doc/arch/observability/observability.md`.

## Clarifications

On 2026-07-23 the scope was narrowed to a headless local vertical slice with
fake providers. Presentation clients, live provider calls, and live MCP
execution were explicitly deferred. No unresolved product ambiguity blocks the
technical plan.
