# Feature 006 Quickstart

## Prerequisites

- Install the official Codex CLI and authenticate it outside Workbench with a
  ChatGPT subscription login (`codex login`).
- Resolve the real versioned executable file. Do not configure a symlink.
- Confirm that the intended account and billing mode are permitted for local
  programmatic `codex exec` use under OpenAI's terms.

## Configure

Add an explicit provider to `.workbench/workbench.yaml`:

```yaml
providers:
  codex:
    type: subscription-cli
    driver: codex
    executable: /absolute/path/to/versioned/codex

models:
  codex-review:
    provider: codex
    runtime_model: gpt-5.4

roles:
  codex-reviewer:
    model: codex-review
```

The executable must be an absolute, current-user-owned, non-writable-by-others
regular file, and none of its path components may be group/world writable.
Workbench never discovers `codex` through `PATH`.

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
  --role codex-reviewer "Inspect the repository and summarize its architecture."
cargo run -p workbench-cli -- status <session-id>
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

Feature 006 permits read-only repository inspection under
`--sandbox read-only` with ephemeral sessions. Native writes, elevated
sandbox, approval bypass, MCP registration, plugins, resume, and interactive
approvals remain unavailable.

## Optional Live Compatibility

The ignored live smoke requires the same real, non-symlink executable path used
by production configuration. It performs `login status` and version identity
checks only. It sends no user message and starts no model turn. Inference must
never be added to the default test target.

```bash
CARGO_NET_OFFLINE=true \
WORKBENCH_CODEX_EXECUTABLE=/absolute/path/to/versioned/codex \
WORKBENCH_CODEX_VERSION=0.145.0 \
cargo test -p workbench-codex --test live_codex --locked \
  -- --ignored --exact exact_profile_probes_without_sending_a_user_message
```

Set `WORKBENCH_CODEX_VERSION` to the exact normalized version selected by the
lock. Run this command only after `codex login status` reports ChatGPT login on
the same host. This is a compatibility-only check; it does not prove the
production lock, digest, snapshot, or executable provenance.
