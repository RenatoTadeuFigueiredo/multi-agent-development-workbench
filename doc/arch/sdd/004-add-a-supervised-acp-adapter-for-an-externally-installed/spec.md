---
id: 019f9454-4e2b-7053-803e-727ca8782543
number: 004
slug: add-a-supervised-acp-adapter-for-an-externally-installed
status: implemented
created_at: 2026-07-24T13:33:00.843538Z
---
# Feature Specification: Add A Supervised Acp Adapter For An Externally Installed

Feature: 004-add-a-supervised-acp-adapter-for-an-externally-installed
Created: 2026-07-24
Related issue: #7

## Objective

Add the first production provider adapter by supervising an externally
installed Grok Build process over its confirmed ACP version 1 stdio contract.
The adapter must preserve the Workbench's durable attempt, cancellation,
security, and workspace-isolation guarantees without embedding Grok Build or
depending on its private implementation.

## Scope

This feature includes executable validation and locking, child-process
supervision, JSON-RPC 2.0 NDJSON transport, ACP initialization and
authentication status, session creation and load, prompt streaming,
cancellation, session updates, normalized failures, adapter health, and a
deterministic offline fake ACP agent.

This feature excludes the Grok-derived terminal fork, the Workbench ACP server
for editors, live MCP forwarding, automatic provider updates, paid-model tests,
ACP pause, Grok-specific `x.ai/*` interjection, and a permission-policy bridge.

## User Stories

- As a SuperGrok user, I want Workbench to use my installed and authenticated
  Grok Build runtime so that I can run Grok-backed work without replacing my
  subscription with API billing.
- As a developer, I want Grok output and lifecycle changes normalized into the
  same durable session history as every other provider so that VS Code and CLI
  clients remain synchronized.
- As an operator, I want the executable and protocol compatibility checked
  before dispatch so that a provider update cannot silently alter active work.
- As a security-conscious user, I want Grok authentication to remain owned by
  Grok Build and every reverse permission request denied until a policy bridge
  exists.

## Functional Requirements

1. **FR-004-001:** An ACP provider configuration MUST identify an executable.
   Workbench MUST resolve it to one canonical regular file owned by the current
   user, reject a symlink, missing, non-executable, or group/world-writable
   target, and launch it directly without a shell.
2. **FR-004-002:** The deterministic repository lock MUST pin the adapter
   protocol, bounded `--version` result, and executable SHA-256. Before spawn,
   startup MUST validate the configured path and permissions, retain a private
   executable snapshot, and fail closed when that snapshot differs from the
   lock. Accepting an updated binary requires explicit lock regeneration.
3. **FR-004-003:** Workbench MUST launch Grok Build with argv
   `grok agent --no-leader stdio` and set
   `GROK_DISABLE_AUTOUPDATER=1` in the child environment. The adapter MUST NOT
   invoke the updater or permit an update during a supervised process.
4. **FR-004-004:** Each supervised child MUST belong to one canonical
   workspace and MUST NOT be shared across workspace identities. The child
   MUST use piped stdin, stdout, and stderr, run with the canonical workspace
   as its working directory, and be reaped after exit or shutdown.
5. **FR-004-005:** The transport MUST be full-duplex JSON-RPC 2.0 with one
   UTF-8 JSON value per newline-delimited frame. Reader and writer progress
   MUST be independent so notifications and reverse requests cannot deadlock
   an active prompt.
6. **FR-004-006:** An encoded inbound or outbound ACP frame MUST NOT exceed
   8 MiB, excluding its newline delimiter. Empty, duplicate-key, invalid UTF-8,
   malformed JSON, invalid JSON-RPC, incomplete, and oversized frames MUST fail
   closed with a bounded normalized error.
7. **FR-004-007:** Before the adapter becomes available, it MUST complete ACP
   `initialize` with `protocolVersion: 1`, inspect advertised capabilities,
   and resolve Grok-owned authentication through the baseline authentication
   method. If optional `agentInfo.version` is advertised, it MUST match the
   version pinned by the executable probe. Its omission is compatible because
   the private executable snapshot is already bound to the probed version and
   SHA-256. These post-spawn checks MUST complete before provider dispatch. An
   incompatible protocol, advertised version, or required capability MUST fail
   daemon startup.
8. **FR-004-008:** The supported ACP baseline MUST cover `initialize`,
   authentication, `session/new`, `session/load`, `session/prompt`,
   `session/cancel`, and `session/update`. Additive unknown fields and
   notifications MUST be ignored safely; unknown data MUST NOT alter
   authorization or lifecycle state.
9. **FR-004-009:** `session/update` notifications MUST be translated into
   normalized acknowledged, content, tool, and progress events. A successful
   prompt response MUST produce exactly one definite terminal result and retain
   the Workbench attempt ID across all normalized events.
10. **FR-004-010:** Workbench MUST persist `dispatch_planned` and
    `dispatch_started` according to the existing persist-before-effect
    contract. A child failure before an external attempt starts is a definite
    provider-availability failure; EOF, crash, malformed output, or transport
    loss after `dispatch_started` and before a definite terminal result MUST
    produce `outcome_unknown` with no automatic retry.
11. **FR-004-011:** `session.cancel` MUST be sent at most once for one active
    attempt. Cancellation is confirmed only when the pending
    `session/prompt` response completes with `stopReason: cancelled`; an
    acknowledgement, successful write, process exit, EOF, error response, or
    silence is not confirmation.
12. **FR-004-012:** If confirmed cancellation is not observed within the
    existing five-second deadline, the session MUST reach `outcome_unknown`,
    block automation, and require explicit human reconciliation. The local
    child MAY be terminated for resource safety without claiming that the
    external outcome is cancelled.
13. **FR-004-013:** ACP reverse permission requests MUST always receive a deny
    decision in this feature. Workbench MUST NOT expose, approve, or execute a
    permission-gated provider action until a later policy bridge binds it to
    durable Workbench approvals.
14. **FR-004-014:** The adapter MUST NOT claim ACP pause support or map
    Workbench pause/redirect controls to Grok-specific `x.ai/*` interjection.
    Those capabilities remain explicitly unavailable in this feature.
15. **FR-004-015:** Grok account login, refresh, session cookies, and tokens
    MUST remain in Grok-owned stores. Workbench MUST NOT read, copy, persist,
    log, export, or place them in command arguments or configuration.
16. **FR-004-016:** Raw provider session identifiers, JSON-RPC payloads,
    stdout, stderr, environment values, prompts, and model output MUST NOT enter
    logs or telemetry. Persisted content continues through the existing
    encrypted event boundary; errors expose only stable categories,
    correlation identifiers, and user-safe remediation.
17. **FR-004-017:** Child shutdown MUST stop accepting new adapter work, close
    stdin, allow a bounded graceful exit, terminate a remaining process, drain
    its pipes, and reap it. No supervised process may survive daemon shutdown.
18. **FR-004-018:** Default automated tests MUST use an explicitly configured
    fake ACP executable, make zero network calls, never discover or execute an
    installed `grok`, require no account, and consume no provider quota.
19. **FR-004-019:** Compatibility MUST be capability-first and
    version-second. A same-major additive update that completes initialization
    and supplies required capabilities MAY run after explicit re-locking; an
    incompatible protocol, known malformed behavior, missing capability, or
    executable digest mismatch MUST remain unavailable.
20. **FR-004-020:** Public adapter health MUST expose only `available` or
    `unavailable`. Authentication-required, incompatibility, spawn, and crash
    causes MUST remain bounded startup or lifecycle errors rather than public
    health states. A later prompt MAY start a fresh child after a definite idle
    failure, but Workbench MUST NOT restart or resume an uncertain active
    attempt automatically.

## Security Requirements

- **Data sensitivity/classification.** Prompts, repository paths, model output,
  tool content, provider session identifiers, authentication state, and child
  diagnostics may contain confidential source or account data. Only normalized
  content required by the session is persisted, through the existing encrypted
  event store; process metadata is retained only in redacted form.
- **Authentication/authorization.** Grok Build owns authentication and its
  credential store. Workbench observes only a bounded authentication status.
  Reverse ACP permission requests cross an untrusted provider boundary and are
  denied unconditionally until a durable policy bridge is specified.
- **Input validation.** The executable, configuration, lock, every JSON-RPC
  envelope, ACP method result, notification, identifier, enum, and frame length
  are untrusted. Canonical-path, ownership, permission, private-snapshot, and
  digest checks happen before spawn. Protocol, authentication, capability, and
  any optionally advertised agent-version checks happen after spawn but before
  availability or dispatch; strict frame validation and the 8 MiB bound happen
  before state changes.
- **Cryptography in transit/at rest.** ACP uses local inherited stdio rather
  than a network listener. Sensitive normalized events continue to use the
  existing per-session envelope encryption at rest. Provider credentials and
  raw session handles are never copied into Workbench storage.
- **Logging/audit.** Audit history records the adapter kind, lifecycle phase,
  bounded outcome, attempt ID, and correlation ID. It excludes argv values
  beyond the fixed profile, environment, JSON-RPC bodies, stdout, stderr,
  prompts, output, provider session identifiers, and authentication details.
- **Error-handling information exposure.** Protocol, spawn, authentication,
  timeout, cancellation, and crash failures map to stable redacted categories.
  Raw OS errors and child diagnostics remain outside client-visible replies and
  telemetry.

## Acceptance Scenarios

1. **Offline ACP execution:** Given a pinned fake ACP executable advertising
   protocol version 1 and available authentication, when a prompt is routed to
   it, then Workbench initializes it, creates a session, streams normalized
   updates, records one attempt, and completes without network or real Grok
   execution.
2. **Pinned launch profile:** Given a valid Grok ACP provider, when its child is
   spawned, then the fake observes argv `agent --no-leader stdio`, environment
   `GROK_DISABLE_AUTOUPDATER=1`, the canonical workspace working directory, and
   no shell intermediary.
3. **Executable replacement:** Given a lock created for one executable digest,
   when the executable changes, then startup fails before spawn until the
   operator explicitly regenerates the lock.
4. **Frame boundary:** Given a negotiated fake agent, when it emits or receives
   a frame of exactly 8 MiB, then the frame is accepted; when the encoded frame
   is one byte larger, it is rejected without unbounded allocation.
5. **Malformed transport:** Given duplicate keys, invalid UTF-8, truncated
   JSON, an invalid JSON-RPC envelope, or an empty line, when the adapter parses
   it, then the child is isolated and the client receives only a stable
   redacted failure.
6. **Pre-dispatch crash:** Given a child exits during initialization, when
   preflight runs, then the adapter is unavailable, no prompt frame is written,
   and no provider attempt starts.
7. **Active crash:** Given `dispatch_started` is durable, when the child exits
   before a terminal prompt result, then the attempt reaches
   `outcome_unknown`, no automatic retry occurs, and prior events remain
   readable.
8. **Confirmed cancellation:** Given an active prompt, when Workbench sends
   `session/cancel` and the prompt completes within five seconds with
   `stopReason: cancelled`, then the session reaches `cancelled`.
9. **Unconfirmed cancellation:** Given an active prompt, when cancellation is
   acknowledged but the prompt hangs, exits, errors, or completes without
   `stopReason: cancelled`, then the session reaches `outcome_unknown` within
   five seconds and requires human reconciliation.
10. **Permission denial:** Given the child sends a reverse permission request,
    when the adapter handles it, then it returns deny, starts no protected
    action, and records no approval claim.
11. **Secret containment:** Given unique secret markers in Grok-owned auth
    state, provider session data, stdout, and stderr, when execution succeeds
    or fails, then those markers are absent from replies, logs, telemetry,
    locks, SQLite, WAL, and exported events.
12. **Compatible update:** Given an explicitly re-locked same-major fake agent
    with additive fields and notifications, when it initializes with all
    required capabilities, then the adapter ignores unknown additions and
    completes the baseline flow.
13. **Incompatible update:** Given an incompatible protocol or missing required
    capability, when initialization completes, then preflight rejects the
    adapter during daemon startup before a prompt is dispatched, and public
    health remains unavailable.
14. **Workspace and shutdown isolation:** Given two workspace daemons and two
    fake ACP children, when one daemon shuts down, then only its child exits and
    is reaped while the other workspace remains available.
15. **Default zero-quota suite:** Given only default test configuration, when
    the complete automated suite runs, then every ACP process is the committed
    fake, no installed `grok` is discovered, and no network or paid model call
    occurs.

## Observability

Existing provider duration and attempt metrics add the bounded
`adapter_kind=acp` dimension and outcomes `success`, `failed`, `cancelled`,
`timeout`, and `outcome_unknown`. Structured logs record spawn, initialization,
authentication-state, prompt, cancellation, crash, and reap phases with stable
categories only. Provider process, session, request, path, version, and model
identifiers are trace attributes or redacted diagnostics, never metric labels.
Raw stdio and environment values are forbidden from every signal.

## Clarifications

- This adapter makes the Workbench daemon an ACP client of the official Grok
  Build provider process. It is separate from the future Workbench ACP server
  and the Grok-derived terminal presentation fork.
- The confirmed provider command is `grok agent --no-leader stdio`; automatic
  updates are disabled through `GROK_DISABLE_AUTOUPDATER=1`.
- ACP version 1 is the compatibility baseline. Capability negotiation decides
  usability after the executable has been explicitly re-locked.
- An ACP cancellation acknowledgement is not a terminal fact. Only the pending
  prompt's `stopReason: cancelled` confirms cancellation.
- Pause and Grok-specific `x.ai/*` interjection are outside this feature.
- All reverse ACP permission requests are denied until a separate feature maps
  them to Workbench policy and durable approvals.
