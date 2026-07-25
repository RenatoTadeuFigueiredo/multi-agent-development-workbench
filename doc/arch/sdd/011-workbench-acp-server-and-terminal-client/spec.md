---
id: 019f9730-b2c3-7d4e-9f5a-0110ac950001
number: 011
slug: workbench-acp-server-and-terminal-client
status: implemented
created_at: 2026-07-25T00:40:00.000000Z
---
# Feature Specification: Workbench ACP Server and Terminal Client

Feature: 011-workbench-acp-server-and-terminal-client
Created: 2026-07-25
Related issue: #17

## Objective

Expose Workbench sessions to ACP-capable clients through a headless ACP agent
stdio bridge (`workbench agent stdio`) that reuses the existing daemon and
versioned local protocol. Ship an offline-proven MVP path and document residual
gaps for the Grok-derived terminal fork.

## Scope

Includes:

- ACP v1 agent stdio surface (`initialize`, `session/new`, `session/prompt`,
  `session/cancel`, `session/update` notifications);
- bridge to workspace-local daemon via the existing NDJSON protocol;
- offline fake ACP client harness and acceptance tests;
- documentation of deferred Grok terminal fork integration.

Excludes:

- embedding or re-spawning Grok Build as the orchestration plane;
- full Grok-derived pager fork and PTY backend (documented residual gap);
- Zed/JetBrains packaging beyond the stdio agent surface;
- provider-native write tools beyond existing gateway policy.

## Functional Requirements

1. **FR-011-001:** `workbench agent stdio` MUST speak JSON-RPC 2.0 NDJSON on
   stdio with an 8 MiB frame ceiling and fail closed on malformed frames.
2. **FR-011-002:** `initialize` MUST advertise `protocolVersion: 1` and agent
   identity `workbench` without requiring a live provider.
3. **FR-011-003:** `session/new` MUST create a Workbench session through the
   daemon protocol and return an ACP session id correlated to it.
4. **FR-011-004:** `session/prompt` MUST forward user text to the daemon and
   stream assistant content as `session/update` notifications, completing with
   a definite stop reason when the daemon attempt completes.
5. **FR-011-005:** `session/cancel` MUST request daemon cancellation for the
   active attempt.
6. **FR-011-006:** Default tests MUST use offline fakes only (in-process or
   local daemon with fake provider) and MUST NOT dial external providers.
7. **FR-011-007:** Documentation MUST list the Grok-derived terminal fork as a
   residual gap when not shipped in this increment.

## Success Criteria

- Offline acceptance harness for Feature 011 is green.
- `workbench agent stdio` handshake and prompt path proven offline.
- STATUS marks 011 delivered or gap-listed; roadmap 001–017 complete or gaps
  explicit.

## Observability

ACP agent stdio surfaces only redacted daemon session and attempt outcomes.
Frame decode failures and oversized frames fail closed with stable error kinds.
No provider credentials, raw tool payloads, or absolute host paths are logged
on the agent stdio path.
