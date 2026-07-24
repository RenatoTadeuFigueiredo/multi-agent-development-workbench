---
status: proposed
date: 2026-07-24
deciders: [maintainer]
consulted: []
informed: []
---

# Supervise the Official Codex CLI Through Exec JSONL

## Context and Problem Statement

The Workbench needs Codex-backed repository review and validation without
embedding a second agent runtime, copying ChatGPT credentials, changing the
provider-independent core, or replacing an operator's selected billing path.
The official Codex CLI exposes non-interactive automation through
`codex exec`, including `--json` newline-delimited event output, sandbox
selection, ephemeral sessions, and ChatGPT subscription authentication owned
by the CLI under `CODEX_HOME`.

The interface is a same-user child process with access to provider-owned
configuration and network services. Its executable can update, its output is
untrusted, and subscription eligibility for programmatic use is controlled by
OpenAI. The integration must therefore be explicit, pinned, bounded,
conservative about authority, and transparent about billing.

## Decision Drivers

- Keep authentication and refresh entirely in the official Codex CLI.
- Preserve Rust as the daemon and adapter implementation language.
- Reuse the existing provider port and durable uncertainty semantics.
- Prevent silent executable changes and silent API-billing substitution.
- Support prompt streaming and cancellation without granting native write,
  shell, MCP, plugin, or elevated-sandbox authority.
- Keep default validation deterministic, offline, and quota-free.
- Mirror the Feature 005 subscription-cli pattern so later drivers remain
  independent.

## Considered Options

- Supervise the installed CLI through `codex exec --json` with a read-only
  sandbox and ephemeral sessions.
- Drive the experimental Codex `app-server` JSON-RPC control plane as the
  primary transport.
- Call the OpenAI API directly from this subscription adapter.
- Drive the interactive Codex TUI through a PTY.

## Decision Outcome

Chosen option: **supervise the installed CLI through `codex exec --json`**,
because it preserves the official credential owner, keeps protocol translation
in an isolated Rust crate, supplies structured JSONL streaming for automation,
and avoids screen scraping, experimental control-plane dependency, or a second
language runtime.

The configured executable is canonicalized, privately snapshotted, probed, and
pinned by version, protocol (`codex-exec-jsonl/1`), and SHA-256. Every prompt
receives a fresh child with `--ephemeral` and `--sandbox read-only`. The child
uses a fixed safe launch profile; future central permission and MCP features
may widen the surface explicitly.

Authentication preflight accepts ChatGPT subscription login status but
Workbench does not initiate login or handle tokens. Inherited API-key and
alternate-provider selectors are removed from the child environment.
Documentation states that OpenAI controls whether programmatic subscription
use is eligible and how it is charged; API-based product use remains a
separate provider path.

### Consequences

- Good: Codex becomes replaceable behind the same Workbench provider contract.
- Good: version locking and capability-first parsing make updates explicit.
- Good: credentials remain outside Workbench storage and process arguments.
- Good: default tests need no Codex installation, account, network, or quota.
- Good: read-only sandbox is expressed by official CLI flags rather than
  reinvented policy.
- Bad: the JSONL event subset is vendor-specific and must track compatible CLI
  releases.
- Bad: a same-user provider process is not an operating-system sandbox.
- Bad: one-shot `exec` cancellation is coarser than a bidirectional control
  protocol; unconfirmed cancellation and `outcome_unknown` remain common.
- Bad: read-only native authority cannot implement repository mutations until
  the permission bridge or shared MCP feature exists.
- Bad: ChatGPT subscription eligibility for automation is external policy and
  can change independently of the repository.
