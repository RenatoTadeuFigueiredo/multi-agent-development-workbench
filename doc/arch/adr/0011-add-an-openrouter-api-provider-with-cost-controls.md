---
status: accepted
date: 2026-07-24
deciders: [maintainer]
consulted: []
informed: []
---

# Add an OpenRouter API Provider with Cost Controls

## Context and Problem Statement

Features 004–006 cover subscription CLI providers. Operators also need
API-billed models through OpenRouter without placing secrets in repositories
or allowing unbounded spend. The configuration model already describes
`type: api` providers with `credential_ref` and privacy flags, but no adapter
implements the provider port or budget enforcement.

## Decision Drivers

- Keep credentials in the OS keychain via opaque `credential_ref` handles.
- Fail closed on missing credential or over-budget before HTTP dispatch.
- Reuse the shared `ProviderAdapter` port and paid-inference attempt rules.
- Keep default tests offline, deterministic, and free of paid calls.
- Prefer OpenAI-compatible Chat Completions streaming for MVP breadth.

## Considered Options

- First-party OpenRouter Chat Completions adapter with config budgets.
- Shell out to a third-party CLI that owns the OpenRouter key.
- Defer API providers until a multi-tenant cloud control plane exists.
- Embed keys in environment variables without budget gates.

## Decision Outcome

Chosen option: **first-party OpenRouter Chat Completions adapter** in
`workbench-openrouter`, composed by the daemon, with `policies.cost` budgets
enforced before dispatch and offline fakes for acceptance.

### Consequences

- Good: API models join the same routing, approval, and audit path as CLIs.
- Good: budgets and missing secrets fail closed without silent spend.
- Good: offline fakes preserve CI zero-quota defaults.
- Bad: process-local spend ledger may reset on daemon restart unless later
  persisted; operators should set conservative session ceilings.
- Bad: Chat Completions MVP does not claim full Responses multi-tool loops.
