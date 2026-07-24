# Implementation Plan: Supervised Grok Build ACP Provider Adapter

## Overview

Add an isolated Rust ACP client adapter that supervises the official Grok Build
executable for one workspace. The adapter implements the existing provider
port, while the core retains routing, policy, persistence, attempt, and
cancellation authority.

## Technical Approach

Create a `workbench-acp` crate for the generic JSON-RPC 2.0 NDJSON codec,
full-duplex request correlation, bounded reverse-request handling, child
supervision, and ACP-to-provider event translation. The Grok launch profile is
an adapter-boundary value: direct executable invocation with arguments
`agent --no-leader stdio` and `GROK_DISABLE_AUTOUPDATER=1`.

Configuration validation requires an executable for ACP providers. Startup
canonicalizes the target, rejects files that are not current-user-owned,
executable, regular, non-symlink, and free of group/world write access, then
retains a private executable snapshot. Lock generation records the bounded
`--version` result and SHA-256; startup verifies the snapshot digest before
spawn. After spawn, ACP initialization verifies protocol, authentication state,
required capabilities, and an optional `agentInfo.version` when advertised
before availability or dispatch. Grok Build 0.2.111 omits `agentInfo`; the
locked probe version and executable digest remain authoritative. Session
overrides may select a pinned adapter but cannot introduce or replace an
executable absent from the base lock.

The supervisor owns stdin, stdout, stderr, pending JSON-RPC requests, and the
child wait handle. Independent reader and writer tasks enforce an 8 MiB
per-frame ceiling. Newline discovery resumes at the previous buffer extent,
avoiding quadratic rescans while accepting an exact 8 MiB frame and rejecting
one byte over the limit. A bounded stderr drain prevents child blockage while
raw diagnostics remain out of logs. Shutdown closes new work, requests bounded
graceful exit, terminates any survivor, drains tasks, and reaps the process.
Children are never shared between canonical workspace identities.

Initialization negotiates ACP `protocolVersion: 1`, required session/prompt
capabilities, and Grok-owned authentication status. The supported baseline is
`initialize`, authentication, `session/new`, `session/load`, `session/prompt`,
`session/cancel`, and `session/update`. Unknown additive fields are ignored
without granting authority. Reverse permission requests always receive deny.
Pause and `x.ai/*` interject are not advertised or translated.

The adapter converts updates to the existing normalized provider stream. Crash,
EOF, malformed frames, and prompt/cancel races are classified relative to the
durable attempt boundary: pre-dispatch failures are definite availability
failures, while loss after `dispatch_started` becomes `outcome_unknown`.
Cancellation becomes definite only when the pending prompt returns
`stopReason: cancelled`. The provider receives a 4.5-second cancellation
budget, preserving 500 milliseconds for durable daemon finalization within the
public five-second deadline.

A small test binary in `workbench-testkit` implements the confirmed ACP profile
with scripted handshake, streaming, malformed-frame, crash, permission, and
cancellation modes. Every automated path names that executable explicitly;
tests do not inspect PATH, launch the installed Grok binary, use credentials,
open a network connection, or consume quota. Live Grok validation remains an
ignored manual handshake-only smoke test.

## Companion Artifacts

- The feature CUE value object records the fixed launch, transport,
  compatibility, permission, cancellation, and framing profile.
- ADR 0005 records the external supervised-process and capability-first
  compatibility decision.
- The feature Gherkin file binds executable pinning, offline streaming,
  malformed input, crash, cancellation, permission, secrecy, update, and
  shutdown behavior. Its 15 headings expand to 23 fingerprinted concrete cases
  whose 11 distinct evidence tests run through the repository-owned acceptance
  harness.
- The privacy threat model records the new child-process disclosure boundary.
- Local operations document authentication ownership, explicit update/re-lock,
  failure recovery, and safe shutdown.
- A separate data model is unnecessary because provider process state remains
  adapter-local and durable lifecycle state uses the existing event model.
