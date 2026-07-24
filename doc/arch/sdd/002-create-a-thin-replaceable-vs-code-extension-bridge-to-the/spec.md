---
id: 019f93fe-0ea3-7072-9ed3-719589bd017f
number: 002
slug: create-a-thin-replaceable-vs-code-extension-bridge-to-the
status: implemented
created_at: 2026-07-24T11:58:48.483683Z
---
# Feature Specification: Create A Thin Replaceable Vs Code Extension Bridge To The

Feature: 002-create-a-thin-replaceable-vs-code-extension-bridge-to-the
Created: 2026-07-24

## User Stories

- As a user I want to attach VS Code to a local Workbench session so that I can
  inspect and control agent work without leaving my editor.
- As a user I want streamed events and Markdown/Mermaid rendering so that I can
  review the work product while it is being produced.
- As a maintainer I want the extension to depend only on the versioned local
  protocol so that the editor surface can be replaced without changing the
  orchestration kernel.

## Functional Requirements

1. The extension shall connect to an already-running local daemon through the
   versioned protocol and negotiate compatibility before sending commands.
2. The extension shall attach to a user-selected session and display its
   replayed and live events, deduplicating stable event identifiers. The local
   protocol intentionally exposes no session-enumeration method.
3. The extension shall send prompts and lifecycle controls through the daemon;
   it shall not implement routing, policy, provider calls, or persistence.
4. The extension shall reconnect after transport loss using the last observed
   event cursor and shall visibly report incompatible or unavailable daemons.
5. Markdown output shall use the VS Code Markdown renderer and Mermaid blocks
   shall be rendered through the extension's replaceable presentation adapter.
6. Tests shall use an offline deterministic fake protocol server and shall not
   require provider credentials, network access, or paid model quota.

## Security Requirements

The bridge handles session content and control requests, so it must not widen
the daemon's existing security boundary.

- **Data sensitivity/classification.** Session prompts, responses, and events
  are potentially sensitive. They remain in daemon storage; the extension only
  holds the active view in memory and does not write transcripts by default.
- **Authentication/authorization.** No new credential surface is introduced;
  the daemon's owner-only local socket and protocol authorization remain the
  sole boundary.
- **Input validation.** Protocol frames, event sizes, cursors, and command
  parameters are validated by the existing Rust codec and daemon contracts.
- **Cryptography in transit/at rest.** Local transport and encrypted daemon
  storage retain their existing protections; the extension adds no plaintext
  persistence.
- **Logging/audit.** Logs contain connection, request, and error metadata only;
  prompt and response bodies are redacted.
- **Error-handling information exposure.** User-facing errors use stable
  categories and omit socket paths, credentials, and response bodies.

## Acceptance Scenarios

- Given a compatible daemon and an existing session
  When the user attaches from VS Code
  Then replayed events appear once and new events stream in order.
- Given an attached session
  When the user submits a prompt
  Then the daemon receives it and the resulting events are rendered.
- Given a dropped socket
  When the daemon becomes available again
  Then the extension reconnects from its last cursor without duplicates.
- Given an incompatible daemon
  When the extension negotiates the protocol
  Then it reports an actionable compatibility error and sends no commands.
- Given Markdown containing a Mermaid block
  When the event is displayed
  Then Markdown is readable and Mermaid is rendered by the presentation layer.
- Given the deterministic fake protocol server
  When the extension test suite runs
  Then it completes offline without credentials or network access.

## Observability

The daemon remains the source of operational telemetry and command latency.
The extension presents bounded, content-free connection and reconnection
notices in the in-memory session document. Request identifiers remain carried
through the protocol and correlatable with daemon traces.

## Clarifications
