# Publication Runbook — Workbench

The current workspace produces a local `workbench` binary containing the
daemon, headless CLI, encrypted storage, versioned Unix protocol, and the
supervised Grok Build ACP provider adapter. The thin VS Code bridge ships from
`extensions/workbench-vscode`. The Grok-derived TUI fork, Workbench ACP server,
Claude/Codex/OpenRouter adapters, and shared MCP runtime remain separate work.

## Purpose

Publish a reviewed, reproducible source or local binary build without implying
that deferred provider, terminal, or MCP integrations exist.

## Trigger

Use this procedure for a reviewed source release or a local validation build.
Do not publish packages or create a release tag unless that action was
explicitly approved.

## Preconditions

- The change is linked to its tracked issue and approved pull request.
- The checkout is the exact reviewed commit and the working tree is clean.
- Rust 1.95.0 and the pinned Speckit revision are installed.
- Linux Secret Service or macOS Keychain is available for the explicit
  platform test.

## Steps

1. Confirm the checkout is clean and points to the reviewed commit.
2. Run the complete offline and platform gates.
3. Build the locked release workspace.

```bash
git status --short
speckit status
make check
make test-platform
cargo build --workspace --release --locked
```

`make check` must pass formatting, Clippy, contract drift, the offline workspace
suite, the Feature 001–004 acceptance profiles, SLOs, analysis, verification,
and validation.
`make test-platform` must run in an expendable unlocked credential-store
context; success is required before claiming support for that operating system.

The release binary is `target/release/workbench`. Verify it with the reviewed
offline configuration:

```bash
target/release/workbench config lock
target/release/workbench config validate
```

Start the daemon in one terminal:

```bash
target/release/workbench daemon
```

Then inspect it from another terminal:

```bash
target/release/workbench --json status
```

Record only sanitized command results; never attach local configuration,
databases, key-store records, or prompt bodies to a release.

## Verification

- `make check` and the target platform credential-store contract pass.
- The release build uses `--locked` and the pinned Rust toolchain.
- The built commit and recorded checksum match the reviewed source.
- Release notes state the implemented/deferred boundary and contain no
  sensitive data.

## Publication

1. Confirm the built commit equals the reviewed commit.
2. Record the Rust version, operating systems, gate results, and artifact
   checksum in the release review.
3. Merge through the approved pull request.
4. Create a source or binary tag only when explicitly authorized.
5. Describe the current boundary accurately; distinguish the implemented Grok
   ACP adapter and VS Code bridge from deferred terminal, provider, ACP-server,
   and MCP work.

## Rollback

Stop when any gate fails. If a bad change reaches `main`, create a focused
revert through the normal review flow; never rewrite shared history. Re-run
`make check` and the affected platform test on the revert, then record the
failure on the tracked issue before republishing.
