# Project Status

Last reviewed: 2026-07-24

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
`40c666d4bf505348e55dca8de4cbb967edf8b7f1` (Features 001–007, including the
central MCP gateway via issue #13 / PR #20). Feature 008 (configurable
multi-agent workflow executor) is in progress on branch
`008-execute-configurable-multi-agent-workflows-that-resolve` (issue #14).

| Feature | Delivered capability | Change |
|---|---|---|
| 001 | Orchestration kernel, encrypted sessions, configuration, protocol, daemon, and headless CLI | Issue #1 / PR #2 |
| 002 | Thin, replaceable VS Code bridge to workspace-local daemon sessions | Issue #3 / PR #4 |
| 003 | Versioned workspace-scoped session discovery and selection | Issue #5 / PR #6 |
| 004 | Supervised Grok Build ACP v1 provider boundary | Issue #7 / PR #8 |
| 005 | Supervised, read-only Claude Code subscription adapter | Issue #9 / PR #10 |
| 006 | Supervised, read-only Codex subscription adapter | Issue #12 / PR #19 |
| 007 | Central MCP lifecycle, pins, allowlists, approvals | Issue #13 / PR #20 |
| 008 | Configurable multi-agent workflow executor (in progress) | Issue #14 |

Features 001–007 completed the Speckit lifecycle through implementation and
are on `main` at the checkpoint above. Feature 008 has completed specify →
clarify → plan → tasks → analyze; domain workflow runtime, config validation,
and offline acceptance harness scaffolding are on the feature branch. Daemon
dispatch loop composition remains to finish.

## Active Work

- **Branch:** `008-execute-configurable-multi-agent-workflows-that-resolve`
- **Issue:** [#14 — Configurable multi-agent workflow executor](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/14)
- **Speckit feature:** `008-execute-configurable-multi-agent-workflows-that-resolve`
- **Phase at last handoff:** implement in progress (T001–T003 partial; daemon
  composition T004–T007 remaining). Next ready product issue after merge is #15.

## Ordered Roadmap

- **In progress:** [#14 — Configurable multi-agent workflow executor](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/14).
- **Next ready after #14 merge:** [#15 — Real-time VS Code workflow controls](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/15).

| Order | Issue | Increment | Dependency |
|---|---|---|---|
| 1 | [#14](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/14) | Configurable multi-agent workflow executor | MCP gateway and governed write tools |
| 2 | [#15](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/15) | Real-time VS Code workflow controls | Stable workflow control protocol |
| 3 | [#16](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/16) | OpenRouter provider and cost controls | Central approval and audit policy |
| 4 | [#17](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/17) | Workbench ACP server and terminal client | Stable workflows and terminal fork spike |

Each product increment requires its own branch from `main`, tracked issue,
active Speckit feature, and completion of the phase returned by `speckit next`.

## Known Gaps

- OpenRouter is not implemented.
- Feature 007 ships the offline MCP gateway (stdio supervision, loopback/fake
  HTTP, pins, allowlists, approvals). Non-loopback HTTPS MCP still fails
  closed until a TLS client is composed; live package registries are opt-in.
- Feature 008 domain runtime and config validation land first; full daemon
  multi-provider dispatch loop, protocol surface, and recovery composition are
  still completing on the feature branch.
- The VS Code extension is a session bridge foundation, not the final workflow
  control room.
- The Workbench ACP server and Grok-derived terminal backend remain pending.
- Claude and Codex provider-native write tools remain disabled; shared tools go
  through the central MCP gateway allowlist.
- The Feature 005 live smoke was skipped because the recorded host did not
  have an authenticated eligible Claude Code installation.
- The Feature 006 live smoke is opt-in and ignored by default
  (`live_codex`, requires authenticated Codex and pinned executable/version).
- Speckit corpus health is 87/100. Validation is green; the score remains
  reduced because its executable registry does not load the external Rust
  acceptance runners.
- An archived accidental draft feature `099-central-mcp-lifecycle-and-tool-permissions`
  exists from a partial specify collision; implement only against Feature 007.

## Maintenance Contract

Every pull request that affects delivered capabilities, dependencies, or
priority must update this file before merge. After merge:

1. confirm the `main` CI run is green;
2. close the delivered issue with the PR, merge commit, and CI evidence;
3. verify that the delivered baseline, known gaps, and next-ready issue remain
   accurate;
4. open a focused follow-up if a post-merge fact makes versioned status stale;
5. verify the merged checkout with `make context`.
