# Claude Code Provider Operations

This runbook covers Feature 005: the supervised, read-only official Claude Code
provider. It does not cover Anthropic API/OpenRouter routes, Claude login,
writable workflows, or shared MCP.

## Ownership and Configuration

Install and authenticate the official Claude Code CLI outside Workbench.
Configure the real versioned executable, not a symlink or `PATH` name:

```yaml
providers:
  claude:
    type: subscription-cli
    driver: claude-code
    executable: /absolute/path/to/versioned/claude

models:
  specification:
    provider: claude
    runtime_model: fable

roles:
  product-architect:
    model: specification
```

The target must be a current-user-owned regular executable. It and its path
components cannot be group/world writable. Workbench never initiates login,
reads the provider credential store, or receives an OAuth token.

## Lock and Start

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- daemon
```

In a separate terminal, inspect adapter health and start a routed session:

```bash
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
cargo run -p workbench-cli -- prompt <session-id> \
  --role product-architect "Inspect the repository and summarize its architecture."
cargo run -p workbench-cli -- status <session-id>
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

The global status must show adapter `claude` as `available` before dispatch.
An unavailable state exposes only a redacted category; provider process data
must not be added to logs.

Lock generation privately snapshots the executable, runs bounded `--version`
and `auth status --json` probes, and pins
`claude-code-stream-json/1`, version, and SHA-256. Authentication must report an
existing first-party Claude subscription login. API-key and alternate cloud
routes are rejected for this provider.

Each prompt gets a fresh child in the canonical workspace. The fixed profile
uses bidirectional stream JSON, partial messages, safe mode, `dontAsk`, no
provider transcript persistence, no Chrome or slash commands, a strict empty
MCP manifest, and only `Read`, `Glob`, and `Grep`.
`DISABLE_AUTOUPDATER=1` is fixed. Inherited API keys, auth tokens, alternate
endpoints, and Bedrock/Vertex/Foundry selectors are removed.

## Billing and Usage Boundary

Anthropic controls authentication eligibility, acceptable use, and billing.
As of the Feature 005 review on 2026-07-24, `claude -p`, Agent SDK, and
third-party application use draw from subscription limits. Anthropic paused
the previously announced separate Agent SDK credit on 2026-06-15. These terms
can change. Workbench does not guarantee that a Claude plan covers an
operation and does not broker Claude subscription access for another user.

For distributed products, API-backed use, or a billing mode not eligible for
this local adapter, use a separately configured Anthropic Console or
OpenRouter provider when that adapter is implemented. Review the current
[Claude Code legal guidance](https://code.claude.com/docs/en/legal-and-compliance)
and [CLI documentation](https://code.claude.com/docs/en/cli-usage) before
production use. Review the
[current plan guidance](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
as well.

## Update and Rollback

Workbench never runs Claude Code install or update commands. To accept an
update:

1. Finish or reconcile active attempts and stop the workspace daemon.
2. Retain the previous executable or the vendor-supported rollback method.
3. Update with the official process outside Workbench.
4. Resolve the new real executable and review version, provenance, permission,
   tool, MCP, plugin, skill, browser, persistence, protocol, and billing
   changes.
5. Regenerate the lock, validate configuration, run `make check`, and inspect
   adapter health before the first prompt.

Restore the previous executable and matching lock if compatibility fails.
Never edit a digest or protocol identity manually.

## Failures and Cancellation

| Condition | Required action |
|---|---|
| Subscription auth unavailable | Authenticate with the official CLI outside Workbench, then restart and re-check. Do not add credentials to configuration. |
| Digest/version mismatch | Confirm the update was intentional and re-lock after review, or restore the pinned executable. |
| Initialization or capability failure | Restore the previous compatible CLI. Do not bypass the lock or safe profile. |
| Crash, EOF, malformed frame, or incomplete cancellation after dispatch | Treat the attempt as `outcome_unknown`; inspect durable history and reconcile manually. Do not retry automatically. |

Cancellation is confirmed only after the correlated interrupt response and a
result with `aborted_streaming` or `aborted_tools`. An acknowledgement, error,
silence, EOF, or process exit alone is insufficient.

## Quota-Free Validation

The default suite runs only the committed fake:

```bash
make test-claude
make check
```

The ignored live smoke performs auth, initialization, and interrupt-receipt
checks only; it sends no user message and starts no model turn:

```bash
CARGO_NET_OFFLINE=true \
WORKBENCH_CLAUDE_EXECUTABLE=/absolute/path/to/versioned/claude \
WORKBENCH_CLAUDE_VERSION=2.1.218 \
cargo test -p workbench-claude --test live_claude --locked -- \
  --ignored --exact exact_profile_initializes_without_sending_a_user_message
```

Replace `2.1.218` with the exact normalized version selected by the lock. The
smoke is compatibility-only; it does not verify the production lock, digest,
snapshot, or provenance. Inference requires separate operator authorization.
Raw stdout, stderr, auth fields, thinking, usage, tool data, provider
identifiers, and environment values must never be attached to issues or
compatibility records.
