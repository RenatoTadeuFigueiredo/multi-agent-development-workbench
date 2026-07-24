---
id: 019f95d0-93c0-77b1-993b-f8c30e2c7c39
number: 006
slug: add-a-supervised-codex-subscription-adapter-that-pins-an
status: implemented
created_at: 2026-07-24T20:28:22.337779Z
---
# Feature Specification: Supervised Codex Subscription Adapter

Feature: 006-add-a-supervised-codex-subscription-adapter-that-pins-an
Created: 2026-07-24
Related issue: #12

## Objective

Add a production `subscription-cli` provider adapter that supervises an
operator-installed official Codex CLI executable through its non-interactive
`codex exec --json` JSONL contract. The adapter must preserve Workbench
durability, workspace isolation, cancellation, executable locking, and
credential ownership while making read-only Codex-backed review and validation
available through the existing provider port.

## Scope

This feature includes explicit executable configuration, safe version and
authentication probes, version and SHA-256 locking, private executable
snapshots, one isolated Codex child per provider attempt, bounded JSONL output
parsing, a fixed read-only sandbox launch profile, stream normalization,
cancellation and process cleanup, crash handling, shutdown, and a
deterministic fake Codex executable.

This feature excludes login, credential transport, API-key or OpenRouter
billing, Claude-specific drivers, shared MCP gateway expansion, native write
or shell tools, interactive tool approvals, provider session persistence and
resume, multi-agent workflow execution, the experimental Codex app-server as a
required transport, the terminal fork, and the Workbench ACP server.

## User Stories

- As a ChatGPT subscriber, I want Workbench to use my already authenticated
  official Codex CLI installation without copying its credentials.
- As a developer, I want Codex to review and validate repository work and
  stream analysis into the same durable history used by other providers.
- As an operator, I want binary changes and incompatible JSONL behavior to
  fail closed until I explicitly regenerate the lock.
- As a security-conscious user, I want Codex-native write, shell, network,
  MCP, plugin, and elevated sandbox surfaces disabled until Workbench can
  govern them.

## Functional Requirements

1. **FR-006-001:** A `subscription-cli` provider using the `codex` driver MUST
   declare an absolute executable path. Workbench MUST resolve it to a
   canonical current-user-owned regular executable, reject symbolic links,
   parent traversal, unsafe writable path components, and group/world-writable
   targets, and invoke it directly without a shell.
2. **FR-006-002:** Explicit lock generation MUST run bounded `--version` and
   `login status` probes against a private snapshot of the configured
   executable. The repository lock MUST pin `codex-exec-jsonl/1`, the
   normalized Codex CLI version, and the executable SHA-256. Startup MUST fail
   before provider dispatch when the configured executable differs from the
   lock.
3. **FR-006-003:** Workbench MUST NOT run `codex update`, `codex login`,
   `codex logout`, or any installer command. An update becomes eligible only
   after an operator replaces the configured executable and explicitly
   regenerates the lock. Auto-update side effects MUST NOT be relied on for
   compatibility.
4. **FR-006-004:** The authentication probe MUST accept only evidence of an
   already established ChatGPT subscription login (for example a bounded
   status reporting ChatGPT login). API-key login, missing login, or unknown
   auth modes MUST leave the provider unavailable. Workbench MUST remove
   inherited API-key, alternate-endpoint, base-URL, and OSS-local provider
   selectors from the supervised child so a configured subscription route
   cannot silently become API or local billing.
5. **FR-006-005:** Workbench MUST NOT offer Codex login, read or copy
   `CODEX_HOME` credential files (including `auth.json`), receive an OAuth
   token or access token, or persist authentication output. Public health
   remains `available` or `unavailable`; remediation directs the operator to
   authenticate with the official CLI outside Workbench.
6. **FR-006-006:** Each prompt MUST use a fresh child scoped to one canonical
   workspace and one Workbench attempt. The launch MUST use `--ephemeral` so
   provider-side session files are not persisted for the attempt. Children and
   handles MUST NOT be shared across workspaces or attempts.
7. **FR-006-007:** The fixed launch profile MUST invoke `codex exec` with
   `--json`, `--ephemeral`, `--sandbox read-only`, the requested runtime
   model, and the canonical workspace as the working root (`-C`). It MUST NOT
   pass `--dangerously-bypass-approvals-and-sandbox`, writable sandbox modes,
   or full-auto escalation flags.
8. **FR-006-008:** Native Codex authority in this feature MUST remain
   read-only. Workspace write, danger-full-access, shell mutation, MCP server
   registration, plugins, browser or computer-use surfaces, and unknown tool
   escalation MUST be absent or denied. The adapter MUST NOT advertise a
   Workbench tool-calling capability or claim that native activity has passed
   a future centralized permission bridge.
9. **FR-006-009:** Output MUST be UTF-8 JSONL with one JSON object per line
   and an 8 MiB encoded frame ceiling excluding the newline. Duplicate keys,
   empty frames, invalid UTF-8, malformed JSON, incomplete frames, invalid
   event shapes, and oversized frames MUST fail closed with a stable redacted
   error.
10. **FR-006-010:** The adapter MUST accept additive unknown fields but only
    normalize a documented, version-pinned subset of `codex exec --json`
    events required for content streaming and terminal completion. Unknown
    event types MUST NOT grant authority or produce terminal success.
11. **FR-006-011:** Assistant text deltas and final text MUST become
    normalized content events without duplicate visible text. Tool or item
    metadata MAY become bounded tool events, but raw tool input, output,
    thinking, usage bodies, provider session identifiers, auth material, and
    protocol frames MUST NOT be persisted or exposed.
12. **FR-006-012:** A successful terminal JSONL completion event that proves
    the turn finished without error MUST produce exactly one definite
    completion. A structured provider error is a definite failure only when it
    proves no external mutation could have occurred; EOF, crash, malformed
    output, or transport loss after `dispatch_started` and before a definite
    result MUST produce `outcome_unknown` with no automatic retry.
13. **FR-006-013:** Cancellation MUST target only the active attempt child.
    Because `codex exec --json` is a one-shot supervised process in this
    feature, confirmation is allowed only when a documented abort or cancelled
    terminal event for that attempt is observed before reaping; otherwise the
    adapter MUST terminate and reap the local process group and return
    unconfirmed.
14. **FR-006-014:** If confirmed cancellation is not observed inside the
    provider's 4.5-second budget, the adapter MUST terminate and reap the
    local child and return unconfirmed. The daemon retains 500 milliseconds to
    make `outcome_unknown` durable within the existing five-second public
    deadline.
15. **FR-006-015:** Stderr MUST be drained through a bounded discard path so
    the child cannot block. Raw stderr, stdout, prompts, repository paths,
    authentication fields, environment values, model output, thinking, usage,
    and provider identifiers MUST be absent from logs and telemetry.
16. **FR-006-016:** Shutdown MUST reject new work, close or terminate every
    active child, allow bounded graceful exit, escalate to kill survivors,
    drain pipes, and reap all processes. A child that cannot be reaped is a
    startup or shutdown failure, not a successful provider outcome.
17. **FR-006-017:** Compatibility MUST be capability-first and version-pinned.
    Additive same-contract releases MAY run only after explicit re-locking and
    successful preflight. Missing required capabilities, changed protocol,
    malformed behavior, or digest mismatch MUST remain unavailable.
18. **FR-006-018:** Default automated tests MUST use only an explicitly
    configured repository fake, make no network calls, discover no installed
    `codex`, read no credential store or `CODEX_HOME` auth files, and consume
    no subscription or API quota. Live validation MUST be an ignored, opt-in,
    prompt-free handshake unless the operator separately authorizes paid
    inference.
19. **FR-006-019:** Workbench documentation MUST disclose that Codex
    authentication and billing rules are provider-controlled, that ChatGPT
    subscription eligibility for programmatic `codex exec` use is determined
    by OpenAI, and that API-key or third-party product use may require separate
    OpenAI API authentication under OpenAI's terms.
20. **FR-006-020:** The experimental Codex `app-server`, interactive TUI,
    `mcp-server`, cloud task browser, and workflow executor MUST remain out of
    scope. This feature uses only the supervised `codex exec --json` path.

## Security Requirements

- **Data sensitivity/classification.** Prompts, repository content, model
  output, tool metadata, account state, provider identifiers, and diagnostics
  may contain confidential source or personal data. Only normalized visible
  content enters the existing encrypted event boundary.
- **Authentication/authorization.** The official Codex CLI owns ChatGPT
  subscription authentication under `CODEX_HOME`. Workbench observes a bounded
  status and never handles credentials or token files. Provider-native
  authority is reduced to read-only sandbox execution; mutation and extension
  surfaces are absent.
- **Input validation.** The executable, probes, lock, environment, every JSONL
  frame, event type, content block, enum, and frame length are untrusted.
  Validation occurs before state changes and allocation remains bounded.
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

1. **Offline read-only execution:** Given a pinned fake Codex executable with
   ChatGPT subscription authentication and required capabilities, when a
   prompt runs, then Workbench launches it read-only, streams normalized text,
   records one attempt, and completes without network or quota.
2. **Pinned launch profile:** Given a valid fake, when it is launched, then it
   observes `exec`, `--json`, `--ephemeral`, `--sandbox read-only`, the
   canonical workspace root, sanitized billing environment, and no shell
   intermediary.
3. **Subscription enforcement:** Given an unauthenticated or API-key auth
   status, when preflight runs, then the adapter remains unavailable before a
   prompt starts and no credential detail is returned.
4. **Executable replacement:** Given a lock for one digest, when the binary
   changes, then startup fails before spawn until explicit re-locking.
5. **Frame boundary:** Given an exact 8 MiB frame, when it is read, then it is
   accepted; an encoded frame one byte larger is rejected without unbounded
   allocation.
6. **Malformed stream:** Given duplicate keys, invalid UTF-8, truncated JSON,
   an empty line, or an invalid event, when parsed, then the child is isolated
   and clients receive only a redacted failure.
7. **Sandbox containment:** Given attempts to request workspace-write,
   danger-full-access, approval bypass, MCP registration, plugins, or unknown
   elevated tools, when the launch profile is applied, then none is available
   or approved and no protected mutation begins.
8. **Normalized streaming:** Given partial and final assistant messages, when
   the stream completes, then visible text is emitted once in order, thinking
   and raw usage are absent, and exactly one terminal completion is durable.
9. **Pre-dispatch failure:** Given a probe or launch failure, when dispatch is
   attempted, then the failure is definite, no user message is claimed as
   externally completed, and no successful provider outcome is recorded.
10. **Active crash:** Given `dispatch_started` is durable, when the child exits
    before a definite result, then the attempt reaches `outcome_unknown` and is
    not retried.
11. **Confirmed cancellation:** Given an active prompt, when a documented abort
    or cancelled terminal event for that attempt is observed before reaping,
    then the session reaches `cancelled` within five seconds.
12. **Unconfirmed cancellation:** Given silence, EOF, crash, or an abort without
    a confirming terminal event, when the provider budget expires, then the
    child is reaped and the session reaches `outcome_unknown`.
13. **Secret containment:** Given unique markers in auth output, environment,
    stderr, tool data, thinking, and provider IDs, when success and failure
    paths run, then markers are absent from replies, telemetry, locks, SQLite,
    WAL, exports, and logs.
14. **Workspace and shutdown isolation:** Given two daemons with active fake
    children, when one stops, then only its children are reaped and the other
    workspace remains available.
15. **Default zero-quota suite:** Given default test configuration, when all
    tests run, then only the committed fake executes and no installed Codex
    binary, credential store, network, subscription credit, or API billing is
    used.
16. **No credential file access:** Given an operator `CODEX_HOME` with auth
    material present, when Workbench probes and runs the adapter, then it never
    opens, copies, or logs credential files.

## Observability

Existing provider duration metrics use the bounded
`adapter_kind=subscription-cli` dimension and existing outcome set. Structured
daemon logs report only stable redacted lifecycle outcomes already emitted by
the application boundary. Provider version, model, executable path, account
method, provider session identifier, workspace path, raw content, and process
I/O are forbidden from every signal.

## Clarifications

- The adapter invokes an already installed official Codex CLI executable; it is
  not an OAuth client, login broker, API proxy, or redistribution channel.
- `subscription-cli` does not guarantee that OpenAI permits every programmatic
  use under a ChatGPT plan. Operators remain responsible for their OpenAI
  agreement and billing eligibility.
- Provider-side session persistence is disabled with `--ephemeral` because
  Workbench owns durable encrypted history. Session resume is not advertised in
  this feature.
- Read-only Codex sandbox execution is a temporary bounded bridge. Centralized
  mutation approvals, shared MCP, writable workflows, and multi-agent orchestration
  require separate features.
- Cancellation is confirmed only by a documented abort or cancelled terminal
  event for the active attempt; process kill alone is unconfirmed.
- Reference host research freezes the initial floor around Codex CLI 0.145.x
  `codex exec --json` behavior; the repository lock pins the exact tested
  executable.
