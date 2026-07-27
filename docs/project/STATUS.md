# Project Status

Last reviewed: 2026-07-27

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
`4f523ad` (PR #41 — STATUS checkpoint after docs 001–016). Feature code
baseline remains gap-zero through Feature 016 (issue #33 / PR #39,
`64315a0`). Public product docs include Mode C operator runbook and published
`GROK_BUILD_FORK_COMPATIBILITY_PIN` (this change).

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
| 015 | Claude/Codex provider-native write tools under central policy | Issue #32 |
| 016 | WorkbenchBackend terminal launch surface (`workbench agent stdio`) | Issue #33 |

Feature 015 adds fail-closed `policies.provider_native_writes` (default
disabled). When `mode: approval-required` and a provider id is allowlisted,
Claude may launch Write/Edit and Codex may use workspace-write sandbox with
`file_change` observation. Shared tools stay on the MCP gateway.

Feature 016 delivers `workbench-terminal-backend::WorkbenchBackend`, the
selectable external ACP backend that plans `workbench agent stdio` with
absolute paths for the Grok-derived terminal integration. The fork
compatibility pin is published for the `WorkbenchBackend` integration commit;
full dual-upstream rebase automation and expanded PTY suite remain residual in
`grok-build`. Mode C runbook:
[`docs/operations/mode-c-grok-tui-workbench.md`](../operations/mode-c-grok-tui-workbench.md).

## Active Work

- **Branch:** `main`
- **Issue:** none — gap-zero monorepo backlog cleared
- **Next ready after merge:** none (maintenance / new roadmap only)

## Ordered Roadmap

- **Planned 001–011 slice:** complete. Gap follow-ups #28–#33 delivered for the
  monorepo scope.

| Order | Issue | Increment | Dependency |
|---|---|---|---|
| done | [#16](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/16) | OpenRouter provider and cost controls | Central approval and audit policy |
| done | [#17](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/17) | Workbench ACP server and terminal client MVP | Stable workflows |
| done | [#30](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/30) | Non-loopback HTTPS MCP TLS client | Feature 007 MCP gateway |
| done | [#28](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/28) | Speckit acceptance-binding inventory (tooling) | Features 001–011, 013 harnesses |
| done | [#29](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/29) | ACP agent stdio attach to running daemon | Feature 011 stdio MVP |
| done | [#31](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/31) | Durable cost ledger + OpenRouter live HTTPS | Feature 010 cost controls |
| done | [#32](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/32) | Claude/Codex provider-native write tools under policy | Feature 007 + 005/006 |
| done | [#33](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/33) | WorkbenchBackend terminal integration MVP | Feature 011/012 ACP stdio |

## Known Gaps

Tracked open issues (gap-zero backlog):

| Gap | Issue | Size | Speckit? |
|---|---|---|---|
| *(none in monorepo)* | — | — | — |

Residual (out of tree, not a monorepo Known Gap):

- Full Grok Build pager dual-upstream rebase automation and expanded PTY
  snapshot suite remain in the `grok-build` repository. This monorepo ships
  the `WorkbenchBackend` launch contract and publishes
  `GROK_BUILD_FORK_COMPATIBILITY_PIN` to the fork `renato/main` WorkbenchBackend
  feat-equivalent after grok-build PR #1 merge
  (`85989c9f543e66387a088fc24d8ea83d9771a7ce`). Mode C
  operator path (Grok TUI → `workbench agent stdio` → daemon, same-session
  VS Code attach):
  [`docs/operations/mode-c-grok-tui-workbench.md`](../operations/mode-c-grok-tui-workbench.md).

Detail:

- Feature 012 attaches production `workbench agent stdio` to the running
  workspace daemon socket (#29). Offline Feature 011 harnesses keep
  `InProcessBackend`.
- Feature 014 durable per-session spend + opt-in OpenRouter live HTTPS are
  delivered (#31). Default CI remains offline fake.
- Feature 007 non-loopback HTTPS MCP TLS is composed (Feature 013 / #30). Live
  package registries and public HTTPS smoke remain opt-in / ignored.
- Feature 015 enables Claude/Codex provider-native writes only under
  `policies.provider_native_writes` allowlist + approval-required mode (#32).
- Feature 016 delivers WorkbenchBackend MVP for terminal integration (#33).
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
