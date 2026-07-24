# Feature 005 Quickstart

## Prerequisites

- Install the official Claude Code CLI and authenticate it outside Workbench.
- Resolve the real versioned executable file. Do not configure a symlink.
- Confirm that the intended account and billing mode are permitted for local
  programmatic use.

## Configure

Add an explicit provider to `.workbench/workbench.yaml`:

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

The executable must be an absolute, current-user-owned, non-writable-by-others
regular file, and none of its path components may be group/world writable.
Workbench never discovers `claude` through `PATH`.

## Lock and Validate

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
make check
```

Lock generation runs only bounded version and authentication probes. It does
not send a model prompt. Re-run `config lock` only after intentionally updating
the configured executable and reviewing compatibility.

## Run

```bash
cargo run -p workbench-cli -- daemon
```

Use another terminal or the VS Code client:

```bash
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
cargo run -p workbench-cli -- prompt <session-id> \
  --role product-architect "Inspect the repository and summarize its architecture."
cargo run -p workbench-cli -- status <session-id>
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

Feature 005 permits read-only repository inspection. Native writes, shell,
browser, skills, plugins, MCP, and interactive approvals remain unavailable.

## Optional Live Compatibility

The ignored live smoke requires the same real, non-symlink executable path used
by production configuration. It performs auth-status, stream initialization,
and interrupt-receipt checks only. It sends no user message and starts no model
turn. Inference must never be added to the default test target.

```bash
CARGO_NET_OFFLINE=true \
WORKBENCH_CLAUDE_EXECUTABLE=/absolute/path/to/versioned/claude \
WORKBENCH_CLAUDE_VERSION=2.1.218 \
cargo test -p workbench-claude --test live_claude --locked \
  -- --ignored --exact exact_profile_initializes_without_sending_a_user_message
```

Set `WORKBENCH_CLAUDE_VERSION` to the exact normalized version selected by the
lock. Run this command only after `claude auth status --json` succeeds for a
first-party subscription login on the same host. This is a compatibility-only
check; it does not prove the production lock, digest, snapshot, or executable
provenance.
