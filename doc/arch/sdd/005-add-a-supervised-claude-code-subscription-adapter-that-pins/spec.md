---
id: 019f94e4-b2c5-7d41-b224-55a93145c8bf
number: 005
slug: add-a-supervised-claude-code-subscription-adapter-that-pins
status: implemented
created_at: 2026-07-24T16:10:43.782238Z
---
# Feature Specification: Supervised Claude Code Subscription Adapter

Feature: 005-add-a-supervised-claude-code-subscription-adapter-that-pins
Created: 2026-07-24
Related issue: #9

## Objective

Add a production `subscription-cli` provider adapter that supervises an
operator-installed Claude Code executable through its bidirectional
`stream-json` contract. The adapter must preserve Workbench durability,
workspace isolation, cancellation, executable locking, and credential
ownership while making read-only Claude-backed analysis available through the
existing provider port.

## Scope

This feature includes explicit executable configuration, safe version and
authentication probes, version and SHA-256 locking, private executable
snapshots, one isolated Claude Code child per provider attempt, bounded NDJSON
input and output, SDK control initialization, read-only native tool policy,
stream normalization, cancellation, crash handling, shutdown, and a
deterministic fake Claude executable.

This feature excludes login, credential transport, API-key or OpenRouter
billing, Codex, shared MCP, native write or shell tools, interactive tool
approvals, provider session persistence, workflow artifact materialization,
the terminal fork, and the Workbench ACP server.

## User Stories

- As a Claude subscriber, I want Workbench to use my already authenticated
  official Claude Code installation without copying its credentials.
- As a developer, I want Claude to inspect a repository and stream analysis
  into the same durable history used by other providers.
- As an operator, I want binary changes and incompatible stream behavior to
  fail closed until I explicitly regenerate the lock.
- As a security-conscious user, I want Claude-native mutation, shell, MCP,
  browser, plugin, and skill surfaces disabled until Workbench can govern them.

## Functional Requirements

1. **FR-005-001:** A `subscription-cli` provider using the `claude-code`
   adapter MUST declare an absolute executable path. Workbench MUST resolve it
   to a canonical current-user-owned regular executable, reject symbolic links,
   parent traversal, unsafe writable path components, and group/world-writable
   targets, and invoke it directly without a shell.
2. **FR-005-002:** Explicit lock generation MUST run bounded
   `--version` and `auth status --json` probes against a private snapshot of
   the configured executable. The repository lock MUST pin
   `claude-code-stream-json/1`, the normalized Claude Code version, and the
   executable SHA-256. Startup MUST fail before provider dispatch when the
   configured executable differs from the lock.
3. **FR-005-003:** Workbench MUST set `DISABLE_AUTOUPDATER=1` for every probe
   and supervised child. It MUST NOT run `claude update`, `claude install`, or
   any login command. An update becomes eligible only after an operator
   replaces the configured executable and explicitly regenerates the lock.
4. **FR-005-004:** The authentication probe MUST accept only a bounded JSON
   object reporting `loggedIn: true` with the Claude subscription login method.
   Workbench MUST remove inherited API-key, alternate-endpoint, and cloud
   provider selectors from the supervised child so a configured subscription
   route cannot silently become API billing.
5. **FR-005-005:** Workbench MUST NOT offer Claude login, read or copy the
   credential store, receive an OAuth token, or persist authentication output.
   Public health remains `available` or `unavailable`; remediation directs the
   operator to authenticate with the official CLI outside Workbench.
6. **FR-005-006:** Each prompt MUST use a fresh child and opaque provider
   session identifier scoped to one canonical workspace and one Workbench
   attempt. Provider-side transcript persistence MUST be disabled. Children,
   handles, and control requests MUST NOT be shared across workspaces or
   attempts.
7. **FR-005-007:** The fixed launch profile MUST use bidirectional
   `stream-json`, verbose structured output, partial messages, the requested
   runtime model, `dontAsk`, and no provider transcript persistence. It MUST
   disable Chrome and slash commands and supply an empty strict MCP
   configuration.
8. **FR-005-008:** The only Claude-native tools available in this feature MUST
   be `Read`, `Glob`, and `Grep`. `Bash`, file mutation, web, browser, MCP,
   plugin, skill, subagent, and unknown tools MUST be absent or denied. The
   adapter MUST NOT advertise a Workbench tool-calling capability or claim that
   native activity has passed a future centralized permission bridge.
9. **FR-005-009:** Workbench MUST send an SDK `initialize` control request and
   require a successful correlated response before sending a user message. It
   MUST require the `system/init` stream event, confirm the locked Claude Code
   version when advertised, and require the interrupt-receipt capability before
   advertising cancellation.
10. **FR-005-010:** Input and output MUST be UTF-8 NDJSON with one JSON object
    per line and an 8 MiB encoded frame ceiling excluding the newline.
    Duplicate keys, empty frames, invalid UTF-8, malformed JSON, incomplete
    frames, invalid message shapes, and oversized input MUST fail closed with a
    stable redacted error.
11. **FR-005-011:** The adapter MUST accept additive unknown fields but only
    normalize the documented `system`, `assistant`, `user`, `stream_event`,
    `result`, `control_request`, `control_response`, and
    `control_cancel_request` envelopes. Unknown envelopes MUST NOT grant
    authority or produce terminal success.
12. **FR-005-012:** Assistant text deltas and final text MUST become normalized
    content events without duplicate visible text. Tool-use and tool-result
    metadata MAY become bounded tool events, but raw tool input, output,
    thinking, usage bodies, provider session IDs, and protocol frames MUST NOT
    be persisted or exposed.
13. **FR-005-013:** A successful non-error `result` with a completed terminal
    reason MUST produce exactly one definite completion. A structured error
    result is a definite provider failure only when it proves no external
    mutation could have occurred; EOF, crash, malformed output, or transport
    loss after `dispatch_started` and before a definite result MUST produce
    `outcome_unknown` with no automatic retry.
14. **FR-005-014:** Cancellation MUST send at most one correlated SDK
    `interrupt` control request for an active attempt. It is confirmed only
    when a successful control response is followed by that attempt's result
    with terminal reason `aborted_streaming` or `aborted_tools`.
15. **FR-005-015:** If confirmed cancellation is not observed inside the
    provider's 4.5-second budget, the adapter MUST terminate and reap the local
    child and return unconfirmed. The daemon retains 500 milliseconds to make
    `outcome_unknown` durable within the existing five-second public deadline.
16. **FR-005-016:** Stderr MUST be drained through a bounded discard path so
    the child cannot block. Raw stderr, stdout, prompts, repository paths,
    authentication fields, environment values, model output, thinking, usage,
    and provider identifiers MUST be absent from logs and telemetry.
17. **FR-005-017:** Shutdown MUST reject new work, interrupt or close every
    active child, allow bounded graceful exit, terminate survivors, drain
    pipes, and reap all processes. A child that cannot be reaped is a startup
    or shutdown failure, not a successful provider outcome.
18. **FR-005-018:** Compatibility MUST be capability-first and version-pinned.
    Additive same-contract releases MAY run only after explicit re-locking and
    successful initialization. Missing required capabilities, changed protocol,
    malformed behavior, or digest mismatch MUST remain unavailable.
19. **FR-005-019:** Default automated tests MUST use only an explicitly
    configured repository fake, make no network calls, discover no installed
    `claude`, read no credential store, and consume no subscription or API
    quota. Live validation MUST be an ignored, opt-in, prompt-free handshake
    unless the operator separately authorizes paid inference.
20. **FR-005-020:** Workbench documentation MUST disclose that Claude Code
    authentication and billing rules are provider-controlled, that current
    `claude -p`, Agent SDK, and third-party application use draws from
    subscription limits, and that distributed or third-party use may require
    Claude Console API authentication under Anthropic's terms.

## Security Requirements

- **Data sensitivity/classification.** Prompts, repository content, model
  output, tool metadata, account state, provider identifiers, and diagnostics
  may contain confidential source or personal data. Only normalized visible
  content enters the existing encrypted event boundary.
- **Authentication/authorization.** The official Claude Code executable owns
  subscription authentication. Workbench observes a bounded status and never
  handles credentials. Provider-native authority is reduced to the three
  read-only repository tools; all mutation and extension surfaces are absent.
- **Input validation.** The executable, probes, lock, environment, every NDJSON
  frame, message type, control identifier, content block, enum, and frame
  length are untrusted. Validation occurs before state changes and allocation
  remains bounded.
- **Cryptography in transit/at rest.** The adapter uses inherited local pipes,
  not a network listener. Normalized sensitive events retain existing
  per-session envelope encryption. Provider credentials and transcripts are
  never copied into Workbench storage.
- **Logging/audit.** Audit records adapter kind, lifecycle phase, attempt and
  correlation identifiers, and stable outcomes. Raw frames, content, account
  state, paths, environment, usage, and diagnostics are forbidden.
- **Error-handling information exposure.** Probe, authentication, protocol,
  permission, timeout, cancellation, and crash errors map to bounded categories
  and user-safe remediation. Child output and OS error strings never cross the
  client boundary.

## Acceptance Scenarios

1. **Offline read-only execution:** Given a pinned fake Claude executable with
   subscription authentication and required capabilities, when a prompt runs,
   then Workbench initializes it, exposes only read tools, streams normalized
   text, records one attempt, and completes without network or quota.
2. **Pinned launch profile:** Given a valid fake, when it is launched, then it
   observes the fixed flags, disabled updater, canonical workspace, sanitized
   billing environment, empty MCP configuration, and no shell intermediary.
3. **Subscription enforcement:** Given an unauthenticated or API-key auth
   status, when preflight runs, then the adapter remains unavailable before a
   prompt starts and no credential detail is returned.
4. **Executable replacement:** Given a lock for one digest, when the binary
   changes, then startup fails before spawn until explicit re-locking.
5. **Initialization correlation:** Given interleaved control and stream
   messages, when initialization completes, then only the matching successful
   response and required init capability unlock the user message.
6. **Frame boundary:** Given an exact 8 MiB frame, when it is read, then it is
   accepted; an encoded frame one byte larger is rejected without unbounded
   allocation.
7. **Malformed stream:** Given duplicate keys, invalid UTF-8, truncated JSON,
   an empty line, or an invalid envelope, when parsed, then the child is
   isolated and clients receive only a redacted failure.
8. **Tool containment:** Given attempts to use write, shell, web, MCP, plugin,
   skill, subagent, or unknown tools, when Claude evaluates its tool surface,
   then none is available or approved and no protected action begins.
9. **Normalized streaming:** Given partial and final assistant messages, when
   the stream completes, then visible text is emitted once in order, thinking
   and raw usage are absent, and exactly one terminal completion is durable.
10. **Pre-dispatch failure:** Given a probe or initialization failure, when
    dispatch is attempted, then the failure is definite, no user message is
    written, and no external attempt is claimed.
11. **Active crash:** Given `dispatch_started` is durable, when the child exits
    before a definite result, then the attempt reaches `outcome_unknown` and is
    not retried.
12. **Confirmed cancellation:** Given an active prompt, when one interrupt
    receives success and the result reports `aborted_streaming` or
    `aborted_tools`, then the session reaches `cancelled` within five seconds.
13. **Unconfirmed cancellation:** Given an interrupt ACK without a confirming
    result, an error, silence, EOF, or crash, when the provider budget expires,
    then the child is reaped and the session reaches `outcome_unknown`.
14. **Secret containment:** Given unique markers in auth output, environment,
    stderr, tool data, thinking, and provider IDs, when success and failure
    paths run, then markers are absent from replies, telemetry, locks, SQLite,
    WAL, exports, and logs.
15. **Workspace and shutdown isolation:** Given two daemons with active fake
    children, when one stops, then only its children are reaped and the other
    workspace remains available.
16. **Default zero-quota suite:** Given default test configuration, when all
    tests run, then only the committed fake executes and no installed Claude
    binary, credential store, network, subscription credit, or API billing is
    used.

## Observability

Existing provider duration metrics use the bounded
`adapter_kind=subscription-cli` dimension and existing outcome set. Structured
daemon logs report only stable redacted lifecycle outcomes already emitted by
the application boundary. Provider version, model, executable path, account
method, provider session identifier, workspace path, raw content, and process
I/O are forbidden from every signal.

## Clarifications

- The adapter invokes an already installed official Claude Code executable; it
  is not an OAuth client, login broker, API proxy, or redistribution channel.
- `subscription-cli` does not guarantee that a provider permits every
  programmatic use. Operators remain responsible for their Anthropic agreement
  and billing eligibility.
- Provider-side session persistence is disabled because Workbench owns durable
  encrypted history. Session resume is not advertised in this feature.
- Read-only Claude-native repository tools are a temporary bounded bridge.
  Centralized mutation approvals, shared MCP, and writable workflows require
  separate features.
- Cancellation is proven only by the correlated interrupt response plus an
  explicit aborted terminal reason.
