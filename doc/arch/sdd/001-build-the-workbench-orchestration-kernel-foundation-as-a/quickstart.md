# Feature 001 Quickstart

This quickstart exercises the local, encrypted fake-provider vertical slice.
It performs no external provider request and consumes no paid quota. Feature
001 supports macOS Keychain and Linux Secret Service; the selected credential
store must be unlocked.

## Build and Configure

From the repository root:

```bash
rustup toolchain install 1.95.0 --profile minimal --component clippy,rustfmt
cargo build --workspace
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- config lock
```

Configuration resolves in this order: safe built-ins, user configuration,
`.workbench/workbench.yaml`, then an explicit absolute path supplied with
`--configuration`. `config lock` writes the exact resolved base lock to
`.workbench/workbench.lock`. The lock is local to the workstation and ignored
by this repository.

## Run the Daemon and Client

Start the daemon in one terminal:

```bash
cargo run -p workbench-cli -- daemon
```

Use another terminal in the same repository:

```bash
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
```

Copy the returned `session_id`, then submit and observe deterministic work:

```bash
cargo run -p workbench-cli -- prompt <session-id> "Review the current change"
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

`session attach` replays events after the exclusive cursor and then follows the
live stream. Pressing Ctrl-C disconnects only that observer. Pause, redirect,
resume, and cancel from any other attached terminal:

```bash
cargo run -p workbench-cli -- session pause <session-id>
cargo run -p workbench-cli -- session redirect <session-id> "Use the validated approach"
cargo run -p workbench-cli -- session resume <session-id>
```

Pass `-` as prompt text to read the body from standard input. Use `--json` for
stable envelopes and `--request-id <uuid-v7>` for replay-safe automation.

## Verify

```bash
make check
```

The default gate is offline. Run `make test-platform` separately only when
using an expendable, unlocked OS credential-store context.
