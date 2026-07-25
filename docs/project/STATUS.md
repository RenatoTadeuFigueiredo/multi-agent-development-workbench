# Project Status

Last reviewed: 2026-07-25

This file is the durable handoff for a fresh working session. Conversational
memory is optional context and must never override the repository, Speckit, or
GitHub.

## Resume Work

From the repository root:

```sh
git fetch --prune
make context
```

Then read `AGENTS.md`, inspect the issue identified as next ready below, and
follow the change-request and Speckit gates before editing product code. If the
reported branch is `main` and behind `origin/main`, update it with
`git pull --ff-only` before creating a new branch. Never pull `main` implicitly
into an existing work branch.

Use this precedence when sources disagree:

1. the checked-out Git branch and working tree;
2. `speckit status` and the versioned `doc/arch/` corpus;
3. GitHub issues, pull requests, and completed CI runs;
4. this roadmap snapshot and the README files;
5. conversational memory only as a non-authoritative hint.

## Delivered Baseline

The last reviewed `main` checkpoint is merge commit
`d9852a7` (issue #29 / PR #36). Features 001–014 are on `main` after this change
(Feature 014 durable cost ledger + OpenRouter live HTTPS; issue #31).

| Feature | Delivered capability | Change |
|---|---|---|
| 001 | Orchestration kernel, encrypted sessions, configuration, protocol, daemon, and headless CLI | Issue #1 / PR #2 |
| 002 | Thin, replaceable VS Code bridge to workspace-local daemon sessions | Issue #3 / PR #4 |
| 003 | Versioned workspace-scoped session discovery and selection | Issue #5 / PR #6 |
| 004 | Supervised Grok Build ACP v1 provider boundary | Issue #7 / PR #8 |
| 005 | Supervised, read-only Claude Code subscription adapter | Issue #9 / PR #10 |
| 006 | Supervised, read-only Codex subscription adapter | Issue #12 / PR #19 |
| 007 | Central MCP lifecycle, pins, allowlists, approvals | Issue #13 / PR #20 |
| 008 | Configurable multi-agent workflow executor | Issue #14 / PR #21 + #22 |
| 009 | Real-time VS Code workflow controls | Issue #15 / PR #23 |
| 010 | OpenRouter API provider with cost controls | Issue #16 / PR #25 |
| 011 | Workbench ACP agent stdio bridge MVP (`workbench agent stdio`) | Issue #17 / PR #26 |
| 012 | ACP agent stdio attach to running daemon endpoint | Issue #29 |
| 013 | Non-loopback HTTPS MCP TLS client (`rustls` + native roots) | Issue #30 |
| 014 | Durable session cost ledger + opt-in OpenRouter live HTTPS | Issue #31 |

Feature 010 adds `workbench-openrouter` Chat Completions offline fake, OS
credential_ref resolution, `policies.cost` fail-closed budgets, daemon API
provider composition, and ignored live smoke.

Feature 011 MVP exposes ACP v1 agent stdio (`workbench agent stdio` via
`workbench-acp-server`) that bridges to the daemon/fake application path
without embedding Grok.

Feature 012 production `workbench agent stdio` attaches to the workspace-local
daemon Unix socket (`DaemonSocketBackend`). Missing daemons fail closed.
`InProcessBackend::offline_fake` remains for Feature 011 offline acceptance.

Feature 013 composes `rustls` / `tokio-rustls` / `rustls-native-certs` in
`workbench-mcp` so pinned non-loopback `https://` MCP endpoints dial over TLS.
Offline fakes and loopback cleartext HTTP remain; an offline local TLS fixture
and an ignored live HTTPS smoke cover the path. Production daemon attach uses
the network HTTP/TLS client.

Feature 014 persists redacted per-session `spend_usd_micros` and restores the
OpenRouter cost ledger via `DurableSpendStore`. Default transport remains
offline fake; `OpenRouterTransport::live_https` is explicit opt-in with an
ignored live smoke.

## Active Work

- **Branch:** `014-durable-cost-ledger-openrouter-live-https`
- **Issue:** [#31](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/31) — Durable cost ledger + OpenRouter live HTTPS
- **Next ready after merge:** gap-zero sequence #32 → #33 (see Known Gaps)

## Ordered Roadmap

- **Planned 001–011 slice:** complete. Residual work is gap follow-ups only
  (see Known Gaps), not new numbered roadmap features.

| Order | Issue | Increment | Dependency |
|---|---|---|---|
| done | [#16](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/16) | OpenRouter provider and cost controls | Central approval and audit policy |
| done | [#17](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/17) | Workbench ACP server and terminal client MVP | Stable workflows |
| done | [#30](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/30) | Non-loopback HTTPS MCP TLS client | Feature 007 MCP gateway |
| done | [#28](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/28) | Speckit acceptance-binding inventory (tooling) | Features 001–011, 013 harnesses |
| done | [#29](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/29) | ACP agent stdio attach to running daemon | Feature 011 stdio MVP |
| done | [#31](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/31) | Durable cost ledger + OpenRouter live HTTPS | Feature 010 cost controls |

## Known Gaps

Tracked open issues (gap-zero backlog):

| Gap | Issue | Size | Speckit? |
|---|---|---|---|
| Claude/Codex provider-native write tools under policy | [#32](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/32) | L | Yes |
| Grok-derived terminal pager WorkbenchBackend | [#33](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/33) | XL | Yes |

Detail:

- **Grok-derived terminal pager fork** remains deferred (#33). Feature 011 ships
  the ACP agent stdio bridge (`workbench agent stdio`) only; it does not modify
  the upstream Grok Build pager or add a WorkbenchBackend PTY path.
- Feature 012 attaches production `workbench agent stdio` to the running
  workspace daemon socket (#29). Offline Feature 011 harnesses keep
  `InProcessBackend`.
- Feature 014 durable per-session spend + opt-in OpenRouter live HTTPS are
  delivered (#31). Default CI remains offline fake.
- Feature 007 non-loopback HTTPS MCP TLS is composed (Feature 013 / #30). Live
  package registries and public HTTPS smoke remain opt-in / ignored.
- Claude and Codex provider-native write tools remain disabled; shared tools go
  through the central MCP gateway allowlist (#32).
- Speckit corpus `verifyHealth` stays 0 because Speckit's executable registry is
  binary-local (ADR-0020) and cannot load external Rust acceptance runners.
  Repository-owned inventory is delivered (#28): `make test-acceptance-bindings`
  maps every committed `.feature` to a `workbench-testkit` harness; authoritative
  offline gate remains `make test-acceptance`.

## Maintenance Contract

Every pull request that affects delivered capabilities, dependencies, or
priority must update this file before merge. After merge:

1. confirm the `main` CI run is green;
2. close the delivered issue with the PR, merge commit, and CI evidence;
3. verify that the delivered baseline, known gaps, and next-ready issue remain
   accurate;
4. open a focused follow-up if a post-merge fact makes versioned status stale;
5. verify the merged checkout with `make context`.
