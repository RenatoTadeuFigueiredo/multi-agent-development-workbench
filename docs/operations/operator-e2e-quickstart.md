# Operator End-to-End Quickstart

English operator guide for the monorepo control plane (Features 001–016).
Use this after a fresh clone or when validating a workstation. For product
status and residual scope, see [`docs/project/STATUS.md`](../project/STATUS.md).

## Prerequisites

- Rust 1.95 toolchain (see workspace `rust-version`)
- Unlocked macOS Keychain or Linux Secret Service for encrypted session keys
- Optional live providers only when intentionally leaving the offline path:
  official Claude Code, Codex, Grok Build logins; OpenRouter API key in the
  OS keychain

Default automation and CI stay **offline and quota-free**. Live provider and
public HTTPS paths are opt-in.

## 1. Build

```bash
make build
# or:
cargo build --workspace
```

## 2. Configuration lock

Resolve built-in defaults, user config, and repository
`.workbench/workbench.yaml`, then write the deterministic workstation lock:

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
```

Notes:

- `.workbench/workbench.lock` is workstation-local and ignored by Git.
- Run `config lock` after installing or updating provider executables so digests
  and capability pins match the binaries on disk.
- Never commit secrets, provider sessions, or the lock file.

Example repository-layer fragment (adjust absolute paths and only declare
providers you have installed):

```yaml
version: 1

providers:
  claude:
    type: subscription-cli
    driver: claude-code
    executable: /absolute/path/to/claude
  codex:
    type: subscription-cli
    driver: codex
    executable: /absolute/path/to/codex
  grok:
    type: acp
    executable: /absolute/path/to/grok
  openrouter:
    type: api
    credential_ref: platform:openrouter

policies:
  default_tool_mode: read-only
  cost:
    max_session_usd_micros: 5000000
    max_attempt_usd_micros: 500000
  # Feature 015 — default is disabled / fail-closed
  # provider_native_writes:
  #   mode: approval-required
  #   allowlist: [claude, codex]
```

Provider-specific runbooks:

- [Claude Code](claude-code-provider.md)
- [Codex](codex-provider.md)
- [Grok ACP](grok-acp-provider.md)
- [OpenRouter](openrouter-provider.md)

## 3. Start the daemon

In one terminal, from the workspace root:

```bash
cargo run -p workbench-cli -- daemon
```

In another terminal:

```bash
cargo run -p workbench-cli -- --json status
```

The daemon owns orchestration, encrypted storage, MCP gateway, workflow
execution, and provider supervision. Clients never embed that logic.

## 4. Create and control a session

```bash
cargo run -p workbench-cli -- --json session create
# copy session_id from the JSON result

cargo run -p workbench-cli -- prompt <session-id> "Review the current change"
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

Controls (idempotent where specified by the local protocol):

```bash
cargo run -p workbench-cli -- session pause <session-id>
cargo run -p workbench-cli -- session redirect <session-id> "Use the validated approach"
cargo run -p workbench-cli -- session resume <session-id>
cargo run -p workbench-cli -- session cancel <session-id>
```

Approvals and reconciliation after uncertain outcomes use
`session approve` and `session reconcile` (see
[`doc/arch/domain/cli-surface.md`](../../doc/arch/domain/cli-surface.md)).

Pass `-` as prompt text to read the body from stdin. Use `--json` for stable
envelopes and `--request-id <uuid-v7>` for replay-safe automation.

## 5. Workflow path

Configurable multi-agent workflows (Feature 008) run inside the daemon from
resolved configuration. Stages emit `routing_planned`, dispatch lifecycle, and
`workflow_transition` events on the session stream.

Operator loop:

1. Ensure workflow definitions and role→model bindings exist in configuration.
2. Lock and validate (`config lock` / `config validate`).
3. Start the daemon.
4. Create a session and submit a prompt (or attach an existing session).
5. Follow events with `session attach` or the VS Code bridge.
6. Resolve human gates with `session approve` when policy requires it.

Offline CI uses deterministic fake providers; live stages require authenticated
subscription CLIs or OpenRouter credentials as configured.

## 6. VS Code attach

1. Build or install the Workbench VS Code extension under
   `extensions/workbench-vscode`.
2. Open the same workspace root the daemon resolved (workspace-scoped
   endpoint isolation — Feature 003).
3. Ensure the daemon is running for that workspace.
4. Use extension commands to create or **Select Session**, then attach and
   follow the Markdown event document.
5. Feature 009 surfaces routing plans, workflow stages, and approvals in the
   document and status bar. No orchestration logic runs in TypeScript.

The extension is a thin protocol client only.

## 7. Agent stdio (ACP bridge)

Features 011–012 expose daemon sessions as an ACP v1 agent on stdio:

```bash
# daemon must already be running for the workspace
cargo run -p workbench-cli -- agent stdio
```

Behavior:

- Speaks JSON-RPC 2.0 NDJSON on stdio (`initialize`, `session/new`,
  `session/prompt`, and related ACP agent methods).
- Attaches to the **running** workspace daemon socket (production path).
- Offline Feature 011 harnesses may use an in-process backend; operators should
  use the daemon-attached path above.

Feature 016 ships `workbench-terminal-backend::WorkbenchBackend`, which plans:

```text
<absolute-workbench-cli> agent stdio
```

with absolute executable and workspace paths for the Grok-derived terminal
integration. The full pager UI remains in the
[grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build) fork;
`GROK_BUILD_FORK_COMPATIBILITY_PIN` in this monorepo is empty until that fork
publishes a pin.

## 8. Cost policy notes

OpenRouter (Features 010, 014):

- Store the API key in the OS keychain under the opaque `credential_ref` handle.
- Local budgets (`policies.cost.max_session_usd_micros`,
  `max_attempt_usd_micros`) fail closed **before** HTTP.
- A durable per-session spend ledger records usage; it is not a payment method
  and does not top up OpenRouter credits.
- Default automated tests use offline fakes only; live HTTPS is opt-in /
  `#[ignore]`.

Subscription routes (Claude, Codex, Grok) bill against provider plans, not
OpenRouter. Do not copy provider credentials into configuration files or the
session database.

Provider-native writes (Feature 015):

- Fail closed by default.
- When `policies.provider_native_writes.mode` is `approval-required` and the
  provider id is allowlisted, Claude may use Write/Edit and Codex may use a
  workspace-write sandbox with `file_change` observation.
- Shared tools that must behave identically across providers stay on the
  central MCP gateway (Features 007, 013).

## Offline vs live paths

| Path | What runs | When to use |
|---|---|---|
| Offline / CI | Deterministic fakes, encrypted storage, protocol, workflows, MCP fakes, agent stdio harnesses | Default development and `make check` / `make test-acceptance` |
| Platform secrets | Real Keychain / Secret Service | Explicit `make test-platform` on an unlocked, expendable store |
| Live providers | Real Claude / Codex / Grok / OpenRouter processes or HTTPS | Manual smoke only; never required for merge gates |
| Live MCP HTTPS | Non-loopback TLS client to real servers | Opt-in; package-registry and public smoke remain ignored by default |

Recommended offline gate:

```bash
make check
make test-acceptance
```

## Related docs

- Feature 001 kernel quickstart:
  [`doc/arch/sdd/001-build-the-workbench-orchestration-kernel-foundation-as-a/quickstart.md`](../../doc/arch/sdd/001-build-the-workbench-orchestration-kernel-foundation-as-a/quickstart.md)
- Local operations:
  [`doc/arch/operations/operations.md`](../../doc/arch/operations/operations.md)
- CLI surface: [`doc/arch/domain/cli-surface.md`](../../doc/arch/domain/cli-surface.md)
- Terminal integration:
  [`docs/architecture/grok-build-terminal-integration.md`](../architecture/grok-build-terminal-integration.md)
