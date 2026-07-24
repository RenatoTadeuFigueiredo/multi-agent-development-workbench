# Implementation Plan: Supervised Codex Subscription Adapter

## Overview

Add an isolated Rust adapter for the official OpenAI Codex CLI. The adapter
implements the existing provider port while the daemon retains routing,
approval, encrypted persistence, attempt, cancellation, and reconciliation
authority. It provides read-only repository analysis and validation through a
pinned `codex exec --json` profile and does not broaden the central tool or MCP
policy surface.

## Technical Approach

Add `driver: codex` to `subscription-cli` provider configuration so Claude Code
and Codex remain independent drivers under the same provider type.
Configuration validation requires a driver and absolute executable for this
provider type. Lock generation snapshots the executable, runs bounded
`--version` and `login status` probes, and pins the driver protocol
(`codex-exec-jsonl/1`), normalized version, and SHA-256. The existing private
snapshot implementation stays protocol-neutral and continues to reject unsafe
paths and replacement races.

Create `workbench-codex` for strict bounded JSONL parsing, Codex event
normalization, process lifecycle, environment sanitization, and provider-port
translation. It depends on `workbench-core`, not daemon, storage, UI, Claude,
or ACP types. The adapter freezes the public `codex exec --json` event subset
documented for automation: `thread.started`, `turn.started`,
`turn.completed`, `turn.failed`, `item.started`, `item.completed`, `item.*`
lifecycle variants required for text and bounded tool naming, and `error`.
Unknown additive fields are ignored only after the enclosing event and required
fields validate. Unknown event types never grant authority or terminal success.

Daemon startup probes ChatGPT subscription authentication and performs a
prompt-free capability preflight against the private executable snapshot
(version identity, required flags, and process lifecycle). The provider
catalog is populated only after that preflight succeeds. Authentication details
are discarded. The adapter never runs `codex login`, `codex logout`,
`codex update`, or any installer command, and never opens `CODEX_HOME`
credential files such as `auth.json`.

Every Workbench attempt gets a fresh child. The direct argv is:

```text
codex exec --json --ephemeral --sandbox read-only -C <workspace> -m <model> <prompt>
```

The canonical workspace is both `-C` and the process working directory. The
profile must not pass `--dangerously-bypass-approvals-and-sandbox`, writable
sandbox modes, `--full-auto`, `--oss`, or resume/session identifiers.
`--ephemeral` prevents provider-side session file persistence for the attempt.
Stdout and stderr are piped; stderr is bounded and discarded. Inherited
`OPENAI_API_KEY`, `CODEX_API_KEY`, alternate base-URL selectors, and OSS/local
provider selectors are removed so a configured subscription route cannot
silently become API or local billing.

The codec reads newline-delimited UTF-8 JSON objects with an 8 MiB encoded
frame ceiling excluding the newline. It rejects invalid UTF-8, duplicate keys,
empty or incomplete frames, malformed JSON, oversized frames, and invalid
envelopes with a stable redacted error. Assistant text from `item.completed`
`agent_message` events becomes normalized content without duplicate visible
text. Reasoning, usage bodies, command arguments/output, file-change payloads,
MCP bodies, provider thread or item identifiers, and process diagnostics never
cross the adapter boundary. Read-only tool or command activity may emit only a
bounded tool name and lifecycle category.

Because `codex exec --json` is a one-shot supervised process in this feature,
cancellation targets only the active attempt child. Confirmed cancellation is
allowed only when a documented abort or cancelled terminal event for that
attempt is observed before reaping. Otherwise the adapter terminates and reaps
the local process group and returns unconfirmed. The adapter uses 4.5 seconds,
leaving 500 milliseconds for the daemon to durably publish cancellation or
`outcome_unknown`. Crash, EOF, malformed output, and cancellation races remain
conservative relative to whether dispatch has started: pre-dispatch failures
are definite availability failures; loss after `dispatch_started` without a
definite terminal event is `outcome_unknown` with no automatic retry.

Provider runtime composition stores ACP, Claude, and Codex adapters behind the
same registry and a small managed-lifecycle enum. Shutdown rejects new work,
closes or terminates every active Codex child, escalates within bounded
deadlines, drains pipes, and proves every child was reaped. A child that
cannot be reaped is a startup or shutdown failure, not a successful provider
outcome.

A repository fake implements version, login-status, streaming, malformed
input, sandbox/tool containment, cancellation, crash, and shutdown modes. The
Feature 006 runner fingerprints all concrete Gherkin cases and invokes only the
fake. An ignored live test is prompt-free: it uses an explicitly configured
executable path, checks `login status` for ChatGPT login and a bounded
`--version` identity, then reaps without starting a model turn. The production
lock path, rather than this compatibility smoke, owns snapshot and digest
verification.

### Testing Strategy

- Unit tests in `workbench-codex` cover frame boundaries, event parsing,
  deduplicated text, auth classification, launch argv, and environment
  sanitization.
- `workbench-testkit` hosts `fake_codex` and a Feature 006 acceptance harness
  that fingerprints the Gherkin corpus and runs only the fake offline.
- Daemon composition tests prove registry coexistence with ACP and Claude,
  lock mismatch fail-closed behavior, and shutdown reaping.
- Default `make check` / acceptance targets remain network-free and
  quota-free. Live Codex validation stays `#[ignore]` and prompt-free.

### Risks

- JSONL event shapes are vendor-specific; additive releases require explicit
  re-lock and capability-first preflight.
- One-shot `exec` cancellation is coarser than Claude's bidirectional control
  protocol; unconfirmed cancellation and `outcome_unknown` will be common.
- Read-only sandbox is provider-enforced authority reduction, not an OS
  isolation boundary for the parent user process.
- ChatGPT subscription eligibility for programmatic `codex exec` is
  OpenAI-controlled policy and can change independently of this repository.
- Inherited API keys and project `.env` loading historically caused silent
  API billing; environment stripping and ChatGPT-only preflight must stay
  fail-closed.

## Companion Artifacts

- `research.md` records the official CLI surface, JSONL events, authentication,
  billing substitution risk, update controls, and rejected transports used to
  freeze the adapter boundary.
- The feature CUE object records the driver, protocol, locked identity, fixed
  launch profile, limits, and authentication ownership.
- ADR 0007 records the supervised `codex exec --json` choice and rejected
  alternatives (`app-server`, direct API, PTY/TUI).
- The Gherkin feature supplies quota-free offline cases for launch profile,
  auth, pinning, frames, malformation, sandbox containment, streaming,
  crash, cancellation, secrecy, and shutdown.
- `quickstart.md` describes explicit executable resolution, lock generation,
  offline validation, and opt-in prompt-free compatibility checking.
- The privacy threat model gains the Codex child, billing-substitution, and
  credential-file boundaries.
- Operations documentation records authentication ownership, explicit
  re-lock, recovery, and safe shutdown for the Codex driver.
- A separate persistent data model is unnecessary: child and stream state are
  adapter-local, while durable state remains in the existing encrypted event
  model.
