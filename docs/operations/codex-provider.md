# Codex Provider Runbook

English operator guide for the supervised Codex subscription-CLI adapter.

## Ownership

- Authentication and ChatGPT subscription eligibility are owned by the official
  OpenAI Codex CLI and OpenAI policy. Workbench never brokers credentials.
- Operators install and authenticate Codex outside Workbench (`codex login`).
- Workbench only supervises an explicitly configured absolute executable.

## Configuration

```yaml
providers:
  codex:
    type: subscription-cli
    driver: codex
    executable: /absolute/path/to/codex
models:
  codex-default:
    provider: codex
    runtime_model: gpt-5
```

Regenerate the repository lock after any executable replacement:

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
```

## Behaviour

- Protocol pin: `codex-exec-jsonl/1`
- Launch profile: `codex exec --json --ephemeral --sandbox read-only -C <workspace> -m <model> <prompt>`
- Auth preflight: bounded `codex login status` must report ChatGPT login
- Billing protection: inherited `OPENAI_API_KEY`, `CODEX_API_KEY`, and alternate
  base-URL / OSS selectors are removed from the child environment
- Workbench never runs `codex login`, `logout`, `update`, or installers
- Workbench never opens `CODEX_HOME` credential files such as `auth.json`

## Live smoke (opt-in, prompt-free)

Default CI does not invoke a real Codex binary. Operators may run:

```bash
WORKBENCH_CODEX_EXECUTABLE=/absolute/path/to/codex \
WORKBENCH_CODEX_VERSION=0.145.0 \
cargo test -p workbench-codex --test live_codex -- --ignored --nocapture
```

This smoke checks version identity and ChatGPT login only. It does not start a
model turn and must not be added to the default suite.

## Recovery

| Symptom | Action |
|---|---|
| Provider unavailable after auth probe | Run `codex login` outside Workbench; confirm `codex login status` shows ChatGPT |
| Digest / lock mismatch | Replace executable intentionally, then regenerate the lock |
| Incompatible protocol or version | Upgrade to Codex CLI ≥ 0.145.0, re-lock, restart daemon |
| Outcome unknown after cancel or crash | Inspect durable attempt state; do not auto-retry mutation-sensitive work |

## Shutdown

Daemon shutdown rejects new work, terminates active Codex children, escalates
within the provider budget, drains pipes, and requires every child to be reaped.
Unreaped children are startup or shutdown failures, not successful provider
outcomes.
