# Publication Runbook — Kernel Foundation

Feature 001 produces a local `workbench` binary containing the daemon,
headless CLI, fake-provider vertical slice, encrypted storage, and versioned
Unix protocol. It does not include the VS Code extension, Grok-derived TUI,
live provider adapters, ACP bridge, or MCP runtime.

## Purpose

Publish a reviewed, reproducible kernel-foundation source or local binary build
without implying that deferred editor or live-provider integrations exist.

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
suite, all 23 Gherkin bindings, SLOs, analysis, verification, and validation.
`make test-platform` must run in an expendable unlocked credential-store
context; success is required before claiming support for that operating system.

The release binary is `target/release/workbench`. Verify it without external
provider traffic:

```bash
target/release/workbench config validate
target/release/workbench config lock
target/release/workbench --json status
```

The last command expects a daemon already running from the same repository and
configuration lock. Record only sanitized command results; never attach local
configuration, databases, key-store records, or prompt bodies to a release.

## Verification

- `make check` and the target platform credential-store contract pass.
- The release build uses `--locked` and the pinned Rust toolchain.
- The built commit and recorded checksum match the reviewed source.
- Release notes state the feature 001 boundary and contain no sensitive data.

## Publication

1. Confirm the built commit equals the reviewed commit.
2. Record the Rust version, operating systems, gate results, and artifact
   checksum in the release review.
3. Merge through the approved pull request.
4. Create a source or binary tag only when explicitly authorized.
5. Describe the feature 001 boundary accurately; do not advertise live model,
   editor, terminal, ACP, or MCP support.

## Rollback

Stop when any gate fails. If a bad change reaches `main`, create a focused
revert through the normal review flow; never rewrite shared history. Re-run
`make check` and the affected platform test on the revert, then record the
failure on the tracked issue before republishing.
