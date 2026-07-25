# Mode C — Grok TUI as Workbench ACP client

English operator guide for connecting the Grok Build terminal (TUI) to the
Workbench control plane. Complements the monorepo
[operator E2E quickstart](operator-e2e-quickstart.md) and the architecture
decision
[Grok Build terminal integration](../architecture/grok-build-terminal-integration.md).

## Operating modes

Grok Build and the Multi-Agent Development Workbench are complementary. They
share the multi-model / multi-runtime pain but sit on different layers. Do
**not** merge the monorepos; compose through the stable ACP stdio contract and
the fork compatibility pin.

| Mode | Client | Orchestration | When to use |
|---|---|---|---|
| **A** — Grok multi-model solo | Grok TUI (`grok`) | In-process Grok shell (providers, subagents, local workflows) | Multi-model chat inside one Grok process; no Workbench daemon required |
| **B** — Workbench solo | VS Code extension and/or `workbench` CLI | Workbench daemon (Claude Code, Codex CLI, Grok ACP provider, OpenRouter, MCP, workflows) | Cross-runtime workflows, policy, cost ledger, editor attach |
| **C** — Combined | Grok TUI as ACP **client** | Workbench daemon via `workbench agent stdio` | Same workspace session from TUI **and** VS Code; orchestration stays in the daemon |

```text
Mode C data path

  Grok TUI (fork WorkbenchBackend)
       |  ACP JSON-RPC / NDJSON over stdio
       v
  workbench agent stdio     # Features 011–012
       |  versioned local protocol
       v
  workbench daemon          # Feature 001 composition root
       |
       +--> Claude Code / Codex / Grok provider / OpenRouter / MCP
```

In Mode C the TUI stops being the agent runtime. It becomes a presentation
client of `workbench agent stdio`. Product workflows, routing, approvals, cost
policy, and provider supervision remain in the daemon.

## Prerequisites

1. **Workbench monorepo built** (or an installed `workbench` CLI binary).
2. **Configuration lock** for the target workspace:
   ```bash
   cd /absolute/path/to/workspace
   workbench config lock
   workbench config validate
   ```
3. **Daemon running** for that same workspace:
   ```bash
   workbench daemon
   ```
4. **Absolute path** to the `workbench` CLI (relative paths fail closed).
5. **Grok Build fork** at or after the compatibility pin
   (`GROK_BUILD_FORK_COMPATIBILITY_PIN` in
   `crates/workbench-terminal-backend/src/lib.rs`), built so the pager binary
   includes selectable `WorkbenchBackend`.
6. **`GROK_HOME` isolation** for the fork (dev profile recommended) so pager
   config, sessions, and credentials stay out of production `~/.grok`:
   ```bash
   export GROK_HOME="${HOME}/.grokdev"
   export GROK_LEADER_SOCKET="${GROK_HOME}/leader.sock"
   export GROK_DISABLE_AUTOUPDATER=1
   mkdir -p "${GROK_HOME}"
   chmod 0700 "${GROK_HOME}"
   ```

Default automation and CI stay offline and quota-free. Live providers are
opt-in only.

## Exact environment and commands

### 1. Start Workbench (workspace A)

```bash
export PATH="/path/to/workbench/bin:${PATH}"   # if needed
cd /absolute/path/to/workspace

workbench config lock
workbench config validate
workbench daemon
```

In another shell, confirm the daemon:

```bash
cd /absolute/path/to/workspace
workbench --json status
```

### 2. Launch Grok TUI in Mode C

All of the following are required to select Workbench (fail closed otherwise):

1. Backend selection:
   - `WORKBENCH_TERMINAL_BACKEND=1` (or `true` / `yes`), **or**
   - `GROK_AGENT_BACKEND=workbench`
2. Absolute Workbench CLI path:
   - `WORKBENCH_EXECUTABLE=/absolute/path/to/workbench`, **or**
   - `--workbench-executable /absolute/path/to/workbench`

Example:

```bash
export GROK_HOME="${HOME}/.grokdev"
export GROK_LEADER_SOCKET="${GROK_HOME}/leader.sock"
export GROK_DISABLE_AUTOUPDATER=1
export WORKBENCH_TERMINAL_BACKEND=1
export WORKBENCH_EXECUTABLE="/absolute/path/to/workbench"

cd /absolute/path/to/workspace
# fork binary name may be `grok` or the pager crate binary in your build
grok --no-leader
```

Or without exporting the executable:

```bash
export GROK_HOME="${HOME}/.grokdev"
export GROK_AGENT_BACKEND=workbench
cd /absolute/path/to/workspace
grok --workbench-executable /absolute/path/to/workbench --no-leader
```

Leader mode is forced off when the Workbench backend is selected (avoid global
leader session sharing across workspaces).

### 3. Child process contract (what the TUI launches)

| Field | Value |
|---|---|
| Program | absolute `WORKBENCH_EXECUTABLE` |
| Argv | `agent` `stdio` |
| cwd | workspace root (same absolute workspace as the daemon) |
| Env | `WORKBENCH_TERMINAL_BACKEND=1` (set on the child by `WorkbenchBackend`) |

This matches monorepo `workbench-terminal-backend::WorkbenchBackend` (Feature
016). Relative executable paths, empty paths, and `..` traversal fail closed.

Equivalent manual smoke (without the TUI):

```bash
cd /absolute/path/to/workspace
/absolute/path/to/workbench agent stdio
```

## Same-session attach: VS Code + TUI

Both clients attach to the **same workspace daemon**:

1. Resolve the same absolute workspace root (Feature 003 workspace-scoped
   endpoint isolation).
2. Ensure one `workbench daemon` is running for that workspace.
3. **VS Code:** open that folder, use the Workbench extension to create or
   **Select Session**, then attach (Feature 002 / 009).
4. **Grok TUI (Mode C):** launch as above from the same workspace cwd with
   Workbench backend selected.
5. Session identity and event history live in the daemon/storage layer. Use
   `workbench session attach <session-id>` or the VS Code bridge to follow the
   same stream the TUI drives through ACP.

```text
VS Code extension ──┐
                    ├── versioned local protocol ──► workbench daemon
Grok TUI ── ACP ──► workbench agent stdio ──────────┘
```

Do not run a second daemon for the same workspace. Do not point the TUI at a
different workspace path than the VS Code window.

## Fail-closed troubleshooting

| Symptom | Likely cause | Operator action |
|---|---|---|
| TUI still uses GrokShell / multi-model solo | Backend env/flag not set | Set `WORKBENCH_TERMINAL_BACKEND=1` or `GROK_AGENT_BACKEND=workbench` **and** an absolute executable |
| "executable path is not absolute" / parent traversal | Relative or `..` path | Pass a canonical absolute `WORKBENCH_EXECUTABLE` |
| `agent stdio` cannot attach | Daemon not running or wrong workspace | Start `workbench daemon` in the **same** absolute workspace; re-lock if needed |
| Config / lock errors on daemon start | Stale or missing lock | `workbench config lock` then `config validate` |
| Provider unavailable | Provider binary or auth missing | Use provider runbooks under `docs/operations/`; authenticate outside Workbench; re-lock |
| VS Code and TUI show different sessions | Different workspace roots or two daemons | Align folder paths; stop extra daemons; reattach |
| Credentials bleed between Grok solo and fork | Shared `~/.grok` | Use isolated `GROK_HOME=~/.grokdev` for the fork |
| Fork binary lacks Workbench backend | Built from pin-less main | Checkout pin SHA or `feature/workbench-backend` (see below), rebuild |
| Dual orchestration confusion | Grok local workflows + daemon workflows both active | Product workflow = daemon only in Mode C; keep Grok shell workflows for Mode A |

Never bypass fail-closed path validation, binary pins, or policy gates by
editing lock files or injecting credentials into repository config.

## Compatibility pin

Monorepo constant:

```text
crates/workbench-terminal-backend/src/lib.rs
GROK_BUILD_FORK_COMPATIBILITY_PIN
```

Current published value is the Grok Build commit that lands selectable
`WorkbenchBackend` (`feat(acp): selectable Workbench external ACP backend` on
branch `feature/workbench-backend`). Operators and the fork sync job update
this SHA when the dual-upstream patch stack is rebased.

Fork-side docs (on that branch): `docs/workbench-backend.md` in
[grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build).

## Residual (out of tree)

- Full dual-upstream rebase automation for the pager fork
- Expanded PTY snapshot suite against the Workbench backend
- Ongoing pin publication after each successful rebase

Those remain in `grok-build`. This monorepo owns the launch contract, daemon
attach path, and Mode C operator documentation.

## Related docs

- [Operator E2E quickstart](operator-e2e-quickstart.md) — lock, daemon, session, VS Code, agent stdio
- [Grok Build terminal integration](../architecture/grok-build-terminal-integration.md) — architecture decision
- [Grok ACP provider](grok-acp-provider.md) — official `grok` **provider** boundary (Mode B/C daemon side; not the TUI)
- [Project status](../project/STATUS.md) — delivered baseline and residual
- Feature 011/012/016 specs under `doc/arch/sdd/`
