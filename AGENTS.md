# Repository Guidelines

## Project

This Rust 1.95 workspace uses Speckit. `doc/arch/` is the product source of
truth.

## Architecture

Crates are bounded: `workbench-core` owns domain ports;
`workbench-config` configuration; `workbench-storage` encrypted SQLite;
`workbench-protocol` local NDJSON; `workbench-acp` Grok supervision;
`workbench-claude` Claude supervision; `workbench-codex` Codex supervision;
`workbench-daemon` composition and IPC; `workbench-cli` the headless client;
and `workbench-testkit` acceptance support.
Specifications live in `doc/arch/`, decisions in `docs/`, and contract tooling
in `scripts/`. The terminal stays in its fork.

## Build, Test, and Development Commands

- `make context` — reconstruct Git, Speckit, GitHub, and roadmap state for a
  session.
- `make build` — compile the complete workspace.
- `make check` — run all deterministic offline gates.
- `make test-acceptance` — run Features 001–006 acceptance harnesses.
- `make test-platform` — exercise the real Keychain or Secret Service adapter;
  requires an unlocked credential store.
- `cargo run -p workbench-cli -- config validate` — validate resolved local
  configuration.

Run `workbench config lock` before `workbench daemon`. The generated base lock
is workstation-local and ignored by Git.

## Session Continuity

Conversational memory is never authoritative. In a fresh session, run
`make context`, read `docs/project/STATUS.md`, and then follow the
in-progress non-`main` branch or next-ready issue it identifies. Live Git and
Speckit state take precedence over status prose. After every merge, update the
delivered baseline, evidence, known gaps, and next-ready issue in the same
change.

## Conventions and Constraints

Use `rustfmt`; deny Clippy warnings and unsafe code. Use `snake_case` for modules
and functions, `PascalCase` for types and traits, and `SCREAMING_SNAKE_CASE` for
constants. Keep provider behavior in adapters, DTOs outside the domain, and
orchestration outside presentation clients. Artifacts and Git history use
English.

## Testing Guidelines

Place unit tests beside modules and cross-crate contracts in `tests/`. Name
tests after observable behavior, such as
`replayed_request_does_not_append_an_event`. Defaults must be offline,
deterministic, and quota-free. Cover failure and recovery paths.

## Spec-first Protocol

Spec-first: feature work follows `speckit status` and only the phase returned
by `speckit next`. Respect the guard policy (`[guard]` in
`doc/arch/speckit.toml`): the spec-scope write gate must never be disabled or
bypassed. A red `speckit validate` blocks commits.

## Commit and Pull Request Rules

Use focused Conventional Commits with `refs #<issue>`. Pull requests must link
the issue and specification, explain risk and rollback, list verification, and
include UI evidence when applicable. Obtain human approval before opening a
pull request. Update `docs/project/STATUS.md` whenever delivery state or roadmap
priority changes. Never commit credentials, provider sessions, local databases,
or generated workstation locks.
