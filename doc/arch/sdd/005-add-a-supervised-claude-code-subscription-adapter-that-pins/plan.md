# Implementation Plan: Supervised Claude Code Subscription Adapter

## Overview

Add an isolated Rust adapter for the official Claude Code CLI. The adapter
implements the existing provider port while the daemon retains routing,
approval, encrypted persistence, attempt, cancellation, and reconciliation
authority. It provides read-only repository analysis and does not broaden the
central tool or MCP policy surface.

## Technical Approach

Add `driver: claude-code` to `subscription-cli` provider configuration so later
subscription CLIs can remain independent. Configuration validation requires a
driver and executable for this provider type. Lock generation snapshots the
executable, runs bounded `--version` and `auth status --json` probes, and pins
the driver protocol, normalized version, and SHA-256. The existing private
snapshot implementation becomes protocol-neutral and still rejects unsafe
paths and replacement races.

Create `workbench-claude` for strict bounded NDJSON, SDK control correlation,
Claude message parsing, process lifecycle, and provider normalization. It
depends on `workbench-core`, not daemon, storage, UI, or ACP types. The adapter
uses `claude-code-stream-json/1` and the current public stream-json/Agent SDK
shapes. Unknown additive fields are ignored only after the enclosing message
and required fields validate.

Daemon startup probes subscription authentication and performs a prompt-free
stream handshake against the private executable snapshot. It requires a
correlated SDK initialize response, `system/init`, and interruption support.
The provider catalog is populated only after that preflight succeeds.
Authentication details are discarded. The adapter never runs login or accepts
credential material.

Every Workbench attempt gets a fresh child with the canonical workspace as its
working directory. The direct argv selects bidirectional stream JSON, verbose
partial messages, the routed model, `dontAsk`, no provider transcript
persistence, no Chrome or slash commands, an empty strict MCP manifest, and
only `Read`, `Glob`, and `Grep`. `DISABLE_AUTOUPDATER=1` is set, while API-key,
alternate-endpoint, and cloud-provider selector variables are removed. Stdin,
stdout, and stderr are piped; stderr is bounded and discarded.

The codec independently services reads and writes and rejects frames over
8 MiB, invalid UTF-8, duplicate keys, empty or incomplete frames, malformed
JSON, and invalid envelopes. Text deltas are emitted as normalized content.
When deltas were observed, final assistant and result copies are not emitted
again. Thinking, raw usage, tool arguments/results, protocol bodies, provider
IDs, and process diagnostics never cross the adapter boundary. Read-only tool
activity may emit only a bounded tool name and lifecycle category.

Cancellation sends one correlated `interrupt` control request. It returns
confirmed only after a successful response and a result terminal reason of
`aborted_streaming` or `aborted_tools`. The adapter uses 4.5 seconds, leaving
500 milliseconds for the daemon to durably publish cancellation or
`outcome_unknown`. Crash, EOF, malformed output, and cancellation races remain
conservative relative to whether the user message was written.

Provider runtime composition stores ACP and Claude adapters behind the same
registry and a small managed-lifecycle enum. Shutdown drains every active
Claude attempt, escalates from stdin close to terminate and kill within bounded
deadlines, and proves every child was reaped.

A repository fake implements version, auth-status, initialize, streaming,
malformed input, tool containment, cancellation, crash, and shutdown modes.
The Feature 005 runner fingerprints all 27 concrete Gherkin cases and invokes
only the fake. An ignored live test is prompt-free: it uses an explicitly
configured executable path, checks auth status, stream initialization, and
interrupt receipt, then reaps it without starting a model turn. The production
lock path, rather than this compatibility smoke, owns snapshot and digest
verification.

## Companion Artifacts

- `research.md` records the official CLI, Agent SDK, authentication, billing,
  update, and legal constraints used to freeze the adapter boundary.
- The feature CUE object records the driver, protocol, locked identity, fixed
  launch profile, limits, and authentication ownership.
- ADR 0006 records the supervised stream-json choice and rejected alternatives.
- The Gherkin feature supplies 16 headings and 27 concrete quota-free cases.
- `quickstart.md` describes explicit executable resolution, lock generation,
  offline validation, and opt-in prompt-free compatibility checking.
- The privacy threat model gains the Claude child, native customization,
  billing-substitution, and programmatic-use boundaries.
- A separate persistent data model is unnecessary: child and control state are
  adapter-local, while durable state remains in the existing encrypted event
  model.
