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
`9383140c3e45851634a7b77064ce345430037e34`.

| Feature | Delivered capability | Change |
|---|---|---|
| 001 | Orchestration kernel, encrypted sessions, configuration, protocol, daemon, and headless CLI | Issue #1 / PR #2 |
| 002 | Thin, replaceable VS Code bridge to workspace-local daemon sessions | Issue #3 / PR #4 |
| 003 | Versioned workspace-scoped session discovery and selection | Issue #5 / PR #6 |
| 004 | Supervised Grok Build ACP v1 provider boundary | Issue #7 / PR #8 |
| 005 | Supervised, read-only Claude Code subscription adapter | Issue #9 / PR #10 |

Features 001–005 completed the Speckit lifecycle through implementation.
Feature 005 merged with all pull-request and post-merge `main` checks green.
There is no active Speckit feature at this checkpoint.

## Ordered Roadmap

- **Next ready:** [#12 — supervised Codex subscription adapter](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/12).

| Order | Issue | Increment | Dependency |
|---|---|---|---|
| 1 | [#12](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/12) | Codex subscription adapter | Proven provider contract |
| 2 | [#13](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/13) | Central MCP lifecycle and tool permissions | Provider capabilities and policy ports |
| 3 | [#14](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/14) | Configurable multi-agent workflow executor | Codex adapter and governed write tools |
| 4 | [#15](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/15) | Real-time VS Code workflow controls | Stable workflow control protocol |
| 5 | [#16](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/16) | OpenRouter provider and cost controls | Central approval and audit policy |
| 6 | [#17](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/17) | Workbench ACP server and terminal client | Stable workflows and terminal fork spike |

Each product increment requires its own branch from `main`, tracked issue,
active Speckit feature, and completion of the phase returned by `speckit next`.

## Known Gaps

- Codex, OpenRouter, and shared MCP production adapters are not implemented.
- Multi-stage workflow execution and correction loops are not implemented.
- The VS Code extension is a session bridge foundation, not the final workflow
  control room.
- The Workbench ACP server and Grok-derived terminal backend remain pending.
- Claude write tools remain blocked on the central permission and MCP gateway.
- The Feature 005 live smoke was skipped because the recorded host did not
  have an authenticated eligible Claude Code installation.
- Speckit corpus health is 87/100. Validation is green; the score remains
  reduced because its executable registry does not load the external Rust
  acceptance runners.

## Maintenance Contract

Every pull request that affects delivered capabilities, dependencies, or
priority must update this file before merge. After merge:

1. confirm the `main` CI run is green;
2. close the delivered issue with the PR, merge commit, and CI evidence;
3. verify that the delivered baseline, known gaps, and next-ready issue remain
   accurate;
4. open a focused follow-up if a post-merge fact makes versioned status stale;
5. verify the merged checkout with `make context`.
