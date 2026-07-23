# Repository Guidelines

## Project Structure & Module Organization

Keep the planned Cargo workspace in `crates/` and the thin VS Code client in
`extensions/vscode/`. Store specifications and decisions in `docs/`, scripts in
`scripts/`, and README media in `assets/`. The terminal UI lives in the separate
Grok Build fork described in
`docs/architecture/grok-build-terminal-integration.md`; do not vendor it here.
Place Rust unit tests beside modules and integration tests in each crate's
`tests/` directory.

## Build, Test, and Development Commands

The Speckit plan must define the toolchain before scaffolding. Expected checks:

- `cargo fmt --all -- --check` — verify Rust formatting.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — lint Rust.
- `cargo test --workspace` — run the Rust suite.

Add extension and fork commands after their toolchains are selected. Test the
terminal through the ACP bridge and a pinned fork revision.

## Coding Style & Naming Conventions

Use `rustfmt` and Clippy defaults. Rust modules and functions use `snake_case`;
types and traits use `PascalCase`. TypeScript values use `camelCase`.

## Testing Guidelines

Cover every behavior change or bug fix. Use fake provider adapters; default
tests must not consume paid models. Contract-test both client protocols. Grok
Build syncs require upstream pager tests, ACP contracts, PTY tests, snapshots,
and a reviewed `git range-diff`.

## Specification & Architecture Gate

Once `doc/arch/speckit.toml` exists, follow the active Speckit phase. Do not
write product code before `speckit next` reaches `implement`, or bypass a
failing `speckit validate`. Workflows, credentials, and policies belong in the
Rust core, not presentation clients. Route stable roles and model aliases
through capability contracts; keep provider-specific logic inside adapters.

The Grok Build fork follows a separate downstream patch-stack policy:
`main` is an exact upstream mirror, `workbench` contains the minimal integration
commits, and feature branches target `workbench`. Never place Workbench changes
on the fork's `main` or move orchestration into the pager.

## Commit & Pull Request Guidelines

Use imperative Conventional Commit subjects, for example
`feat: add workflow session view`. Pull requests must link the issue, explain
the change, list verification and risks, and include screenshots for UI work.

Upstream-sync pull requests must name both Grok Build commits, include the
`git range-diff`, and report pager and Workbench backend tests. Never
auto-merge them.

## Repository Identity & Confidentiality

Use `Renato Figueiredo <renato.tadeu.figueiredo@gmail.com>` for commits. Do not
reference employers, clients, or unrelated organizations. Never commit secrets,
API keys, provider sessions, or local session databases.
