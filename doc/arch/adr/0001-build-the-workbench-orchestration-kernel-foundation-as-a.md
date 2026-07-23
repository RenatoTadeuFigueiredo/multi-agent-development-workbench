---
status: proposed
date: 2026-07-23
deciders: [Renato Figueiredo]
consulted: []
informed: []
---

# Provider-Independent Orchestration Kernel

## Context and Problem Statement

Claude, Codex, Grok, and API-backed models expose different authentication,
session, tool, and streaming behavior. If an editor or terminal owns those
differences, workflows, permissions, and history diverge between interfaces.
The project needs one authority that can coordinate providers while allowing
clients and model assignments to change independently.

## Decision Drivers

- The same session must be observable and controllable from VS Code and the
  terminal.
- Native provider subscriptions and optional API access must coexist.
- Models and providers must be replaceable through configuration and adapters.
- Routing, permissions, history, and MCP behavior must be consistent.
- Users must see and interrupt work before an incorrect workflow compounds.
- Default tests must be deterministic and must not consume paid model quota.

## Considered Options

1. A provider-independent local Rust daemon with thin clients and isolated
   provider adapters.
2. VS Code as the orchestration authority with a terminal companion.
3. Provider-specific scripts that pass Markdown artifacts between native
   clients.

## Decision Outcome

Chosen option: **a provider-independent local Rust daemon with thin clients and
isolated provider adapters**. This is the only option that gives every
interface the same durable state and policy decisions without coupling the
product to one editor or provider.

### Consequences

- Good: VS Code, terminal, headless CLI, and ACP clients share one source of
  session truth.
- Good: role and model changes remain configuration concerns, while protocol
  differences stay inside testable adapters.
- Good: permissions, MCP grants, routing decisions, and audit events are
  consistent across providers.
- Bad: the daemon and local protocol add lifecycle, compatibility, and
  migration responsibilities.
- Bad: native provider features are limited to capabilities represented by the
  shared contract or explicit optional extensions.
- Bad: a daemon failure affects every attached client, requiring durable event
  recovery and clear health diagnostics.
