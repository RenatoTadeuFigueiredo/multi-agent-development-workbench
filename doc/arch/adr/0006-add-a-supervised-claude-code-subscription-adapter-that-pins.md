---
status: accepted
date: 2026-07-24
deciders: [maintainer]
consulted: []
informed: []
---

# Supervise the Official Claude Code CLI Through Stream JSON

## Context and Problem Statement

The Workbench needs Claude-backed repository analysis without embedding a
JavaScript or Python agent runtime, copying credentials, changing the
provider-independent core, or replacing an operator's selected billing path.
Claude Code exposes a bidirectional NDJSON interface through
`--input-format stream-json` and `--output-format stream-json`, plus an SDK
control protocol for initialization, permission requests, and interruption.

The interface is a same-user child process with access to provider-owned
configuration and network services. Its executable can auto-update, its output
is untrusted, and subscription OAuth has provider policy constraints for
third-party products. The integration must therefore be explicit, pinned,
bounded, conservative about authority, and transparent about billing.

## Decision Drivers

- Keep authentication and refresh entirely in the official Claude Code CLI.
- Preserve Rust as the daemon and adapter implementation language.
- Reuse the existing provider port and durable uncertainty semantics.
- Prevent silent executable changes and silent API-billing substitution.
- Support prompt streaming and confirmed interruption without granting native
  write, shell, browser, plugin, skill, or MCP authority.
- Keep default validation deterministic, offline, and quota-free.

## Considered Options

- Supervise the installed CLI through its bidirectional stream-json contract.
- Embed the Python or TypeScript Claude Agent SDK as a sidecar.
- Call the Anthropic API directly from this subscription adapter.
- Drive the interactive terminal UI through a PTY.

## Decision Outcome

Chosen option: **supervise the installed CLI through its bidirectional
stream-json contract**, because it preserves the official credential owner,
keeps protocol translation in an isolated Rust crate, supplies structured
streaming and interrupt correlation, and avoids screen scraping or a second
language runtime.

The configured executable is canonicalized, privately snapshotted, probed, and
pinned by version, protocol, and SHA-256. Every prompt receives a fresh child
with provider transcript persistence and automatic updates disabled. The child
uses a fixed safe launch profile with only `Read`, `Glob`, and `Grep`; future
central permission and MCP features may widen the surface explicitly.

Authentication preflight accepts a Claude subscription login but Workbench
does not initiate login or handle tokens. Inherited API and alternate-provider
selectors are removed from the child environment. Documentation states that
Anthropic controls whether programmatic subscription use is eligible and how
it is charged; API-based product use remains a separate provider path.

### Consequences

- Good: Claude becomes replaceable behind the same Workbench provider contract.
- Good: version locking and capability negotiation make updates explicit.
- Good: credentials remain outside Workbench storage and process arguments.
- Good: default tests need no Claude installation, account, network, or quota.
- Bad: the stream-json subset is vendor-specific and must track compatible CLI
  releases.
- Bad: a same-user provider process is not an operating-system sandbox.
- Bad: read-only native tools cannot implement changes until the permission
  bridge or shared MCP feature exists.
- Bad: subscription eligibility and Agent SDK credits are external policy and
  can change independently of the repository.
