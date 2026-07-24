# Research: Codex Exec JSONL Adapter

## Confirmed Boundary

Codex CLI `0.145.0` is installed on the reference macOS host
(`codex-cli 0.145.0`). Its non-interactive surface documents:

- `codex exec` for automation, with aliases and resume/review subcommands that
  remain out of scope for this feature;
- `--json` for JSON Lines event streaming on stdout;
- `--ephemeral` to avoid persisting session rollout files;
- `--sandbox read-only|workspace-write|danger-full-access`;
- `-C` / `--cd` for the agent working root;
- `-m` / `--model` for model selection;
- `--dangerously-bypass-approvals-and-sandbox` as an explicitly dangerous
  escape hatch that Workbench must never pass; and
- default read-only sandbox behavior for `codex exec` when no writable mode is
  requested.

Official non-interactive documentation freezes the automation event family as
JSONL objects whose `type` values include `thread.started`, `turn.started`,
`turn.completed`, `turn.failed`, `item.*`, and `error`. Item types include
agent messages, reasoning, command executions, file changes, MCP tool calls,
web searches, and plan updates. Sample shapes:

```json
{"type":"thread.started","thread_id":"..."}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"...","status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"..."}}
{"type":"turn.completed","usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}
```

Workbench freezes protocol identity `codex-exec-jsonl/1` over this public
subset. It does not depend on the experimental `app-server` JSON-RPC control
plane, the interactive TUI, `mcp-server`, cloud task browser, or multi-agent
workflow executor.

## Authentication and Billing

Codex owns login and stores credentials under `CODEX_HOME` (including
`auth.json`). On the reference host, `codex login status` reports:

```text
Logged in using ChatGPT
```

API-key login is a distinct mode (for example "Logged in using an API key").
Feature 006 accepts only ChatGPT subscription evidence from a bounded
`login status` probe. Missing login, API-key login, or unknown modes leave the
provider unavailable before prompt dispatch.

Historical Codex behavior shows that inherited `OPENAI_API_KEY` / project
`.env` material can silently route usage to API billing even when a ChatGPT
session exists. Automation docs also document `CODEX_API_KEY` for single-run
API auth. The adapter therefore removes API-key, alternate-endpoint, and
OSS/local provider selectors from the supervised child environment and never
passes credential values on argv.

Workbench must not open, copy, or log `auth.json` or other credential files.
Public health remains `available` or `unavailable`. Remediation directs the
operator to authenticate with the official CLI outside Workbench.

OpenAI controls whether programmatic `codex exec` use is eligible under a
given ChatGPT plan and how it is charged. Workbench does not implement login,
promise subscription eligibility, or act as a credential broker. General
API-backed OpenAI product use belongs on a separate API provider path.

## Update and Compatibility

Codex ships `codex update`. Workbench never invokes it. Compatibility is
capability-first after the operator replaces the configured executable and
explicitly regenerates the lock. Private snapshots and pinned version plus
SHA-256 prevent an on-disk update from changing an active daemon without
re-lock.

The initial research floor is Codex CLI `0.145.x` because that is the
confirmed host baseline for `exec --json`, `--ephemeral`,
`--sandbox read-only`, `-C`, and ChatGPT `login status` output. The
repository lock pins the exact tested executable, not merely the minor line.

## Permission Decision

The current Workbench core cannot round-trip a provider-native permission
request through a durable mid-turn approval. This feature therefore launches
only with `--sandbox read-only`, refuses approval-bypass and writable sandbox
flags, does not register MCP servers or plugins, and does not advertise
Workbench tool-calling authority derived from native Codex tools. Write,
shell mutation, network-enabled sandboxes, plugins, browser/computer-use, and
shared MCP authority remain separate features.

## Cancellation Decision

Unlike Claude Code stream-json, `codex exec --json` is a one-shot process
without a documented bidirectional interrupt control request in this surface.
Confirmed cancellation therefore requires a documented abort or cancelled
terminal event for the active attempt before reaping. Process kill alone is
unconfirmed and becomes `outcome_unknown` after the provider budget when no
confirming terminal event arrived.

## Rejected Alternatives

| Option | Why rejected for Feature 006 |
|--------|------------------------------|
| Experimental `codex app-server` | JSON-RPC control plane is experimental relative to `exec`; higher surface and protocol churn |
| Direct OpenAI API from this adapter | Wrong credential owner; bypasses ChatGPT CLI subscription path |
| Interactive TUI over PTY | Screen scraping is unstable; not machine-readable automation |
| Shared Claude/Codex crate | Drivers must stay independent under `subscription-cli` |

## Primary Sources

- <https://learn.chatgpt.com/docs/non-interactive-mode>
- <https://learn.chatgpt.com/docs/auth>
- Local reference host: `codex-cli 0.145.0`, `codex login status` → ChatGPT
- Local reference host: `codex exec --help` (flags freeze for launch profile)
