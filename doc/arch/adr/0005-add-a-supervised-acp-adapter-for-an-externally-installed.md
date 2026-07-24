---
status: accepted
date: 2026-07-24
deciders: [workbench-maintainers]
consulted: []
informed: []
---

# Supervise Grok Build as an External ACP Provider Process

## Context and Problem Statement

Workbench needs a real Grok provider that can use the user's existing
subscription authentication while preserving the provider-independent core.
The confirmed Grok Build boundary is ACP version 1 over full-duplex JSON-RPC
2.0 NDJSON stdio. Embedding Grok internals or screen-scraping its terminal UI
would couple orchestration to an unstable implementation and make updates,
cancellation, security, and testing difficult to reason about.

## Decision Drivers

- Keep Grok authentication and account material owned by Grok Build.
- Preserve durable Workbench attempts, cancellation, and uncertain outcomes.
- Allow Grok Build updates without exposing vendor branches to the core.
- Prevent automatic updates from changing an active provider process.
- Bound every untrusted child-process frame and diagnostic stream.
- Test every default path offline without credentials, network, or paid quota.
- Fail closed for provider-initiated permissions until Workbench policy can
  authorize them durably.

## Considered Options

- Supervise the external Grok Build ACP process behind a generic Rust adapter.
- Embed or fork the Grok provider runtime inside the Workbench daemon.
- Drive the interactive Grok terminal through PTY output and keystrokes.
- Require one exact Grok Build release and reject all other versions.

## Decision Outcome

Chosen option: **supervise the external Grok Build ACP process behind a generic
Rust adapter**. Workbench launches the pinned executable directly as
`grok agent --no-leader stdio`, sets `GROK_DISABLE_AUTOUPDATER=1`, negotiates
ACP `protocolVersion: 1`, and uses capabilities rather than vendor-specific
implementation details to decide whether the runtime is usable.

The ACP transport is JSON-RPC 2.0 with newline-delimited UTF-8 JSON frames and
independent reader and writer progress. The baseline covers initialization,
authentication, session new/load, prompt, cancel, and update. Every frame is
bounded to 8 MiB. Raw provider sessions and stdio never cross the adapter
boundary.

Grok-owned authentication remains untouched. Reverse permission requests are
always denied until a later feature connects them to Workbench's durable policy
and approval model. Cancellation is confirmed only by the outstanding prompt
ending with `stopReason: cancelled`; every other ambiguous result follows the
existing five-second `outcome_unknown` rule.

Compatibility is capability-first, while executable changes remain explicit:
protocol, the bounded `--version` result, and SHA-256 are pinned in the
deterministic lock. An optional ACP `agentInfo.version` is checked when
advertised but is not required by the observed Grok Build 0.2.111 handshake. A
compatible update is accepted only after re-locking and a successful handshake.
Workbench never invokes the Grok updater.

### Consequences

- Good: Workbench can use SuperGrok subscription authentication without
  handling credentials or switching to API billing.
- Good: Provider protocol changes remain isolated in one replaceable adapter.
- Good: Fake subprocess tests exercise real framing, crash, cancellation, and
  shutdown behavior without executing Grok.
- Good: Explicit executable pins and disabled auto-update make sessions
  reproducible and updates reviewable.
- Bad: The daemon must supervise another process and handle full-duplex reverse
  requests, backpressure, and cleanup.
- Bad: A changed executable blocks startup until the operator regenerates the
  lock and compatibility succeeds.
- Bad: Provider-requested permissions are denied, so workflows requiring them
  remain unavailable until the policy bridge is implemented.
- Bad: ACP pause and Grok-specific interjection are unavailable in this
  increment.

### Rollback

Stop the daemon, restore the previously pinned Grok executable and matching
repository lock, then restart and verify adapter health. Active attempts that
lost their child remain `outcome_unknown` and require human reconciliation;
rollback never rewrites their durable history.
