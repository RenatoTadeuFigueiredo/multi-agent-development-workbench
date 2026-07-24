---
status: proposed
date: 2026-07-24
deciders: [maintainer]
consulted: []
informed: []
---

# Centralize MCP Lifecycle and Tool Permissions in the Daemon Gateway

## Context and Problem Statement

Features 004–006 connect Grok, Claude, and Codex through supervised provider
adapters while deliberately disabling provider-local MCP registration and
native mutation. Feature 001 already defines configuration registries for
tools and MCP servers, effect classes, monotonic policy intersection, and
approval protocol events, but it does not launch or supervise live MCP
servers.

Without a central gateway, each provider would install different MCP versions,
apply incompatible permission models, and leave incomplete audit trails.
Cross-provider tools such as shared repository, tracker, or operations helpers
would diverge by adapter. The product architecture requires one daemon-owned
MCP and tool gateway with pinned identity, role and workflow allowlists,
approval gates, and redacted audit.

## Decision Drivers

- Keep MCP ownership in the Rust daemon, not in provider CLIs or the terminal.
- Reuse Feature 001 registries, effect classes, policy intersection, and
  approval protocol instead of inventing a parallel permission system.
- Pin MCP package, image, or executable identity in the deterministic lock.
- Support both supervised local stdio servers and remote HTTP servers.
- Preserve offline, deterministic, quota-free default tests.
- Avoid expanding into workflow execution, OpenRouter, VS Code control room,
  or provider-native write bridges in the same change.

## Considered Options

- Daemon-owned central MCP gateway with lock pins, allowlists, and approvals.
- Allow each provider adapter to install and configure its own MCP servers.
- Defer all shared tools until a multi-agent workflow executor exists.
- Proxy every tool call through a single remote multi-tenant MCP service.

## Decision Outcome

Chosen option: **daemon-owned central MCP gateway**, because it matches the
constitution (daemon owns orchestration, policy, storage, provider lifecycle,
and MCP management), reuses existing configuration and attempt contracts, and
gives every compatible provider the same governed surface.

The gateway:

1. loads `mcp_servers` from resolved configuration and verifies lock pins;
2. supervises stdio children and dials pinned HTTP endpoints;
3. resolves tools of kind `mcp` through intersecting role, workflow, user,
   repository, session, and effect-class policy;
4. blocks sensitive and mutating calls on the existing approval flow;
5. emits redacted attempt and tool lifecycle facts;
6. cancels and reaps gateway-owned work without claiming uncertain success.

Provider-native tools remain outside the gateway until a later explicit bridge.
Repository configuration may narrow grants but never widen user-global policy.

### Consequences

- Good: one pinned MCP version and policy model for all compatible providers.
- Good: approvals and audit reuse the durable session protocol already shipped.
- Good: default tests can use offline stdio/HTTP fakes without network or
  credentials.
- Good: Features 004–006 stay read-only at the provider boundary while shared
  capabilities become available through governed tools.
- Bad: stdio supervision and HTTP pinning add operational surface that must
  fail closed on digest or endpoint drift.
- Bad: redaction means clients see less raw tool detail than provider-native
  UIs; debugging relies on bounded categories and operator-side server logs.
- Bad: without the later workflow executor, multi-step correction loops still
  cannot orchestrate gateway tools across roles automatically.
