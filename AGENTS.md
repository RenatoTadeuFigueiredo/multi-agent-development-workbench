# Repository Guidelines

## Project

This repository is a Rust 1.95 Cargo workspace managed with Speckit.
`doc/arch/` is the product source of truth.

## Architecture

Domain rules and ports live in `crates/workbench-core`; layered configuration
in `workbench-config`; encrypted SQLite persistence in `workbench-storage`;
protocol DTOs and NDJSON framing in
`workbench-protocol`; daemon composition and Unix IPC in `workbench-daemon`;
the headless client in `workbench-cli`; and deterministic fakes and acceptance
tests in `workbench-testkit`. Product specifications are under `doc/arch/`,
supporting design notes under `docs/`, and generated contract checks under
`scripts/`. The Grok-derived terminal remains in its separate fork.

## Build, Test, and Development Commands

- `make build` — compile the complete workspace.
- `make check` — run formatting, Clippy, contracts, tests, acceptance, SLO, and
  Speckit gates.
- `make test-acceptance` — bind and execute all 23 feature 001 scenarios.
- `make test-platform` — exercise the real Keychain or Secret Service adapter;
  this intentionally requires an unlocked OS credential store.
- `cargo run -p workbench-cli -- config validate` — validate resolved local
  configuration without starting the daemon.

Run `workbench config lock` before `workbench daemon`. The generated base lock
is workstation-local and ignored by Git.

## Conventions and Constraints

Use `rustfmt`; Clippy warnings are denied. Keep unsafe code forbidden. Name Rust
modules and functions `snake_case`, types and traits `PascalCase`, and constants
`SCREAMING_SNAKE_CASE`. Keep provider-specific behavior at adapter boundaries,
protocol DTOs out of the domain, and presentation clients free of orchestration
logic. Comments, code, documentation, commits, and pull requests use English.

## Testing Guidelines

Place unit tests beside their module and cross-crate contracts in `tests/`.
Name tests after observable behavior, such as
`replayed_request_does_not_append_an_event`. Default tests must be deterministic,
offline, and use fake providers; never consume paid quota. Add failure-path and
recovery coverage for persistence, protocol, or lifecycle changes.

## Spec-first Protocol

Spec-first: feature work follows `speckit status` and only the phase returned
by `speckit next`. Respect the guard policy (`[guard]` in
`doc/arch/speckit.toml`): the spec-scope write gate must never be disabled or
bypassed. A red `speckit validate` blocks commits.

## Commit and Pull Request Rules

Use focused Conventional Commits with `refs #<issue>`. Pull requests must link
the issue and specification, explain risk and rollback, list verification, and
include UI evidence when applicable. Obtain human approval before opening a
pull request. Never commit credentials, provider sessions, local databases, or
generated workstation locks.
