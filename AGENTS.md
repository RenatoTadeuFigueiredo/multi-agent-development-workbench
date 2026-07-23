# Repository Guidelines

## Architecture & Project Structure

No source tree exists yet. After Speckit reaches implementation, use a Cargo
workspace for the Rust control plane and a thin TypeScript VS Code extension.
Keep specifications in `doc/arch/`, supporting notes in `docs/`, scripts in
`scripts/`, and media in `assets/`. The terminal UI belongs in the separate fork described in
`docs/architecture/grok-build-terminal-integration.md`; do not vendor it here.

## Build, Test, and Development Commands

The Speckit plan must define the toolchain before scaffolding. Expected checks:

- `cargo fmt --all -- --check` — verify Rust formatting.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — lint Rust.
- `cargo test --workspace` — run the Rust suite.

Add client commands after their toolchains are selected.

## Coding Style & Naming Conventions

Use `rustfmt` and Clippy defaults. Rust modules and functions use `snake_case`;
types and traits use `PascalCase`. TypeScript values use `camelCase`.

## Testing Guidelines

Cover every behavior change. Default tests use fake adapters and consume no
paid models. Contract-test client protocols. Grok Build syncs require upstream
tests, ACP and PTY coverage, snapshots, and a reviewed `git range-diff`.

## Spec-first Protocol

Spec-first: `doc/arch/` is the product source of truth. Begin every change with
`speckit status`, run the phase recommended by `speckit next`, and do not write
product code before the active feature reaches `implement`. The configured
guard is the spec-scope write gate; never disable or bypass it to write outside
the active phase. A failing `speckit validate` blocks commits.

Workflows, credentials, and policies belong in the Rust core. Presentation
clients stay thin, and provider-specific logic stays inside adapters.

The Grok Build fork follows a separate downstream patch-stack policy:
`main` is an exact upstream mirror, `workbench` contains the minimal integration
commits, and feature branches target `workbench`. Never place Workbench changes
on the fork's `main` or move orchestration into the pager.

## Commit & Pull Request Guidelines

Use imperative Conventional Commits, for example `feat: add session view`.
Pull requests link the issue, explain changes, list verification and risks, and
include screenshots for UI work.

Upstream-sync pull requests must name both Grok Build commits, include the
`git range-diff`, and report pager and Workbench backend tests. Never
auto-merge them.

## Repository Identity & Confidentiality

Use `Renato Figueiredo <renato.tadeu.figueiredo@gmail.com>` for commits. Do not
reference employers, clients, or unrelated organizations. Never commit secrets,
API keys, provider sessions, or local session databases.
