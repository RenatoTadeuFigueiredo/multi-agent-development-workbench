---
id: 019f908d-d49d-71c1-8822-0fc4ad0f0069
number: 001
slug: build-the-workbench-orchestration-kernel-foundation-as-a
status: analyzed
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
deterministic lock generation, routing decisions, provider and client
contracts, encrypted durable session events, policy decisions, and
fake-adapter contract tests on macOS and Linux.

This feature excludes production provider authentication and model calls, the
VS Code extension, the Grok Build fork, a complete terminal interface, remote
or mobile access, Windows transport, live MCP server execution, and OpenRouter
billing. Later features must implement those capabilities against the
contracts established here.

## User Stories

- As a developer, I want one prompt to resolve to one visible role and provider
  so that I know where work is being performed.
- As a developer, I want multiple protocol clients to observe and control the
  same durable session so that later VS Code and terminal clients can share
  state without owning orchestration.
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
    ordered at-least-once event replay from a cursor. Events MUST carry stable
    identifiers and monotonic session sequence numbers so clients can
    deduplicate them. An incompatible major version MUST fail with a stable
    compatibility error.
12. **FR-012:** Sessions MUST support `pause`, `resume`, `cancel`, and
    `redirect`. Controls MUST be idempotent, durably recorded, and visible to
    every attached client. A paused session MUST start no new provider or tool
    action. Redirect MUST be accepted only while paused or awaiting user input
    and MUST append instruction rather than rewrite history.
13. **FR-013:** Cancellation MUST request adapter cancellation and reject new
    work. Confirmed cancellation reaches `cancelled`; lack of confirmation
    within five seconds reaches `outcome_unknown`, blocks automatic progress,
    and requires explicit human reconciliation.
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
18. **FR-018:** `.workbench/workbench.lock` MUST be deterministic and MUST pin
    the non-session resolved configuration hash, protocol version, adapter
    versions and executable digests, runtime model identifiers, and MCP
    versions and checksums. Each session MUST retain a deterministic lock
    snapshot after overrides and link it to the base lock hash. Neither form
    may contain timestamps or credential values, and session overrides MUST
    NOT introduce an executable or MCP absent from the base lock.
19. **FR-019:** Sensitive event and artifact payloads MUST be encrypted at rest
    with a unique per-session data-encryption key. That key MUST be wrapped by
    a root key held in macOS Keychain or Linux Secret Service. Persistent mode
    MUST fail closed when the platform key store is unavailable.
20. **FR-020:** Every external provider or tool side effect MUST have a stable
    attempt identifier and persisted `planned`, `started`, and terminal or
    `outcome_unknown` events. Only an operation classified as
    `idempotent-read` and explicitly declared idempotent MAY retry
    automatically before dispatch is proven to have started. Paid inference,
    every write or mutation, production access, credential access, and unknown
    operations MUST NOT retry automatically after an uncertain result.
21. **FR-021:** Protocol frames MUST NOT exceed 8 MiB. Each client subscription
    MUST have a bounded queue of at most 1,024 events or 8 MiB, whichever is
    reached first. A slow client MUST be disconnected with `client_lagged`
    without blocking the daemon and MAY resume from its last cursor.
22. **FR-022:** Feature 001 MUST support same-user local IPC on macOS and Linux
    only. Peer ownership and endpoint permissions MUST be verified before
    protocol negotiation; inability to verify either MUST fail closed.
23. **FR-023:** Session history MUST have no automatic expiration by default
    and MUST support configurable retention. Deletion MUST perform
    cryptographic erasure by removing the platform-stored session-key envelope
    and evicting every in-memory copy before purging ciphertext. A durable,
    non-sensitive journal MUST make interrupted deletion resumable and MUST
    become a tombstone at completion. Portable exports MUST use the age v1
    encrypted file format; plaintext export is outside this feature.
24. **FR-024:** Approval resolution MUST identify the pending approval and
    record the actor and `grant` or `deny` decision before any protected action
    starts. Repeated resolution MUST return the recorded outcome; a conflicting
    second decision MUST fail with `invalid_transition`.
25. **FR-025:** For every state-changing command, `request_id` MUST be a durable
    idempotency key, daemon-scoped for `session.create` and otherwise
    session-scoped. Repeating an identical command MUST return its recorded
    outcome without duplicating state or effects. Reusing that ID within its
    scope with different method or parameters MUST fail with
    `invalid_request`. Read and connection commands MAY reexecute to return
    current state.

## Clarified Decisions

- The daemon is local-only for this increment. macOS and Linux are supported;
  Windows and every remote transport are separate features.
- Tests use two generic protocol clients. VS Code and terminal clients remain
  out of scope.
- SQLite is the initial durable event store; the domain contract must not
  expose SQLite-specific types.
- Event delivery is at-least-once. Stable event identifiers and monotonic
  session sequence numbers make duplicate replay safe.
- `pause` stops new orchestration actions but does not discard already received
  provider output. `redirect` requires a paused or awaiting-input session,
  appends new instruction, and never rewrites history.
- A five-second cancellation deadline produces `outcome_unknown` when the
  adapter cannot confirm cancellation. The daemon never converts uncertainty
  into a false success or cancellation.
- Only operations explicitly classified as idempotent may retry automatically.
  Feature 001 further restricts automatic retry to `idempotent-read`; paid
  inference, every write or mutation, production operations, and credential
  access always require human resolution after uncertainty.
- Sensitive persisted payloads are envelope-encrypted. Each session has a data
  key wrapped by a root key. The root key and wrapped session-key envelopes
  reside in macOS Keychain or Linux Secret Service; SQLite stores only key
  identifiers.
- Session history is retained until explicit deletion unless a user configures
  a retention period. Portable exports are always age v1 encrypted.
- The deterministic lock file is part of feature 001 even though live MCP and
  production provider execution are not. `.workbench/workbench.lock` covers
  built-in, user, and repository layers; session overrides produce a linked
  per-session lock snapshot without rewriting the repository lock.
- Coordinator-assisted classification is represented by an adapter contract
  and tested with a fake coordinator; no real model is called.
- The local protocol uses newline-framed JSON with an 8 MiB frame ceiling and
  bounded per-client event queues.

## Security Requirements

- **Data sensitivity/classification:** Prompts, repository paths, tool
  arguments, provider metadata, and artifacts can contain confidential source
  or personal data. Persistent snapshots and audit fields must be redacted by
  schema, not by best-effort string replacement.
- **Authentication/authorization:** The local daemon accepts only same-user
  clients after peer ownership and endpoint permissions are verified. Provider
  credentials remain in provider-owned stores or the operating-system keychain
  and are referenced by opaque identifiers.
- **Input validation:** Configuration, protocol frames, model output, tool
  events, file paths, and cursor values are untrusted. Parsers must reject
  unknown schema majors, traversal, oversized frames, invalid transitions, and
  unresolved aliases before changing state.
- **Cryptography in transit/at rest:** Local transport uses operating-system
  access controls. Sensitive payloads use XChaCha20-Poly1305 with a fresh nonce
  and authenticated session/event metadata. Per-session data keys are wrapped
  by a root key in the platform key store. Portable exports use age v1.
  Credential values are never stored in events, locks, snapshots, or exports.
- **Logging/audit:** Audit events record actor, action, target, decision, result,
  sequence, and correlation identifiers. They omit credentials and default to
  metadata rather than prompt or model-output bodies.
- **Error-handling information exposure:** User-facing failures expose stable
  categories and remediation while sensitive provider diagnostics remain
  redacted and locally restricted.

## Acceptance Scenarios

1. **Encrypted end-to-end execution:** Given valid configuration and an
   available platform key store, when a client submits an explicitly routed
   prompt, then the daemon records input before dispatch, emits one routing
   plan, persists planned, started, and terminal facts under one fake-provider
   attempt ID, encrypts the events, and completes the session.
2. **Configuration precedence and lock:** Given conflicting user, repository,
   and session values, when configuration resolves, then the session value
   wins, the redacted snapshot records source and hash, the session lock links
   to the unchanged base lock, and two equivalent resolutions produce
   byte-identical session locks.
3. **Invalid configuration:** Given an invalid higher-precedence value, when
   configuration resolves, then validation fails without falling back or
   creating a session.
4. **Explicit routing:** Given an explicit target, when routing runs, then one
   routing plan selects that target and the coordinator is not invoked.
5. **Low-confidence routing:** Given no deterministic route and coordinator
   confidence below threshold, when routing runs, then no provider receives the
   prompt and clarification is recorded.
6. **Capability fallback:** Given an incompatible primary provider and a
   compatible configured fallback, when preflight runs, then the fallback is
   visible in the routing plan before dispatch.
7. **Capability rejection:** Given an incompatible provider and no compatible
   fallback, when preflight runs, then dispatch stops with
   `capability_unavailable`.
8. **Shared controls:** Given two protocol clients on one session, when one
   pauses and resumes it, then both observe the same ordered control events and
   no new action starts while paused.
9. **Redirect history:** Given a paused session, when a client redirects it,
   then the instruction is appended, both clients observe it, and earlier
   history remains byte-identical.
10. **Replay deduplication:** Given sequence 20 is the client's last durable
    cursor and event 21 was received but not checkpointed before disconnect,
    when replay strictly after sequence 20 returns events 21 through 25, then
    stable event identifiers allow the client to retain one ordered copy of
    each event.
11. **Protocol negotiation rejection:** Given an incompatible major or
    unauthorized peer, when the client connects, then the daemon fails closed
    with the corresponding stable error and changes no session state.
12. **Oversized frame rejection:** Given an authorized protocol v1 client, when
    it sends a frame larger than 8 MiB, then the daemon returns
    `frame_too_large` and changes no session state.
13. **Slow client isolation:** Given a client exceeds 1,024 queued events or
    8 MiB, when more events arrive, then only that client receives
    `client_lagged` and disconnects while the session and other clients
    continue.
14. **Confirmed cancellation:** Given a provider confirms cancellation within
    five seconds, when the user cancels, then the session reaches `cancelled`
    and preserves prior history.
15. **Unknown outcome:** Given a provider does not confirm cancellation within
    five seconds, when the deadline expires, then the session reaches
    `outcome_unknown`, starts no retry, and waits for explicit human
    reconciliation; a human retry creates a new attempt linked to the uncertain
    attempt.
16. **Monotonic policy:** Given repository configuration grants a globally
    denied tool, when policy resolves, then the tool remains denied and the
    authoritative policy is audited.
17. **Protected-action approval:** Given a production tool passes routing and
    preflight, when the action is proposed, then an approval is recorded and
    the tool receives no call until a human grants it; denial pauses the
    session without invoking the tool.
18. **Encrypted persistence:** Given sensitive prompt and provider output, when
    storage files and WAL pages are inspected without platform keys, then
    neither plaintext payload is recoverable.
19. **Key-store failure:** Given the platform key store is unavailable, when a
    client requests persistent session creation, then creation fails with
    `key_store_unavailable` and no plaintext fallback is created.
20. **Retention policy:** Given terminal sessions with default and configured
    retention, when maintenance crosses the configured deadline, then the
    default-retention session remains and only the configured session enters
    the deletion state machine.
21. **Deletion and export:** Given a retained session, when the user
    exports and deletes it, then the export is age v1 encrypted, the wrapped
    session-key envelope and in-memory key are removed, a deletion tombstone is
    durable, and the deleted payload cannot be decrypted from remaining
    database pages.
22. **Request replay:** Given a prompt was accepted but its reply was lost, when
    a client repeats the same request ID and parameters, then it receives the
    recorded result and no second input or provider attempt is created; reuse
    with changed parameters fails.
23. **Offline tests:** Given default test configuration, when the complete suite
    runs, then only fake adapters and in-memory key stores are invoked, with no
    network request or paid quota use.

## Requirement Traceability

| Requirements | Acceptance scenarios | Governing artifacts |
|---|---|---|
| FR-001, FR-005, FR-008 | 1, 4 | `workbench-local-protocol.yaml`, `session-event.schema.json` |
| FR-002, FR-003, FR-004, FR-018 | 2, 3 | `workbench-configuration.schema.json`, `workbench-lock.schema.json` |
| FR-006, FR-007 | 4, 5 | feature routing CUE schema |
| FR-009, FR-010 | 1, 6, 7 | `provider-capabilities.schema.json` |
| FR-011, FR-021, FR-022 | 10, 11, 12, 13 | `workbench-local-protocol.yaml` |
| FR-012, FR-013 | 8, 9, 14, 15 | `session-lifecycle.md` |
| FR-014, FR-015, FR-024 | 6, 7, 16, 17 | configuration and routing schemas, `workbench-local-protocol.yaml` |
| FR-016 | 7, 11, 12, 15 | protocol error taxonomy |
| FR-017 | 23 | fake-adapter and network-denial test contracts |
| FR-019, FR-023 | 1, 18, 19, 20, 21 | ADR-0002, `session-key-envelope.schema.json`, and `session-event.schema.json` |
| FR-020 | 1, 14, 15 | `session-event.schema.json`, `session-lifecycle.md` |
| FR-025 | 22 | `workbench-local-protocol.yaml`, `local-protocol-semantics.md` |

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
