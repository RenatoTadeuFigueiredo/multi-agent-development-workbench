# Contributing

Thank you for helping improve the Multi-Agent Development Workbench.

## Before Starting

1. Read `AGENTS.md` and the English `README.md`.
2. Search existing issues before proposing a change.
3. Open or reference an issue that explains the problem and expected outcome.
4. Create a focused branch from `main`.

Once Speckit is initialized, product features and behavior changes must follow the active Speckit feature and its current phase. Do not implement product code before the workflow reaches `implement`, and do not bypass a failing `speckit validate`.

## Development Standards

- Keep application logic and first-party binaries in Rust.
- Make surgical changes and avoid unrelated refactors.
- Format Rust with `cargo fmt`.
- Run `cargo clippy --all-targets --all-features -- -D warnings`.
- Run the relevant tests and, when available, the complete workspace suite.
- Add tests for behavior changes, failures, and meaningful edge cases.
- Keep the English and PT-BR README versions aligned when changing shared content.
- Never commit credentials, provider sessions, API keys, or local databases.

Commands may evolve while the project is scaffolded. Prefer the stable entry points documented in `README.md` and `AGENTS.md`.

## Pull Requests

Use a concise Conventional Commit subject. Pull requests should:

- Explain the problem and chosen solution.
- Link the related issue or specification.
- List validation performed.
- Describe security, compatibility, migration, and rollback considerations.
- Include screenshots or recordings for visible interface changes.
- Remain narrowly scoped and ready for review.

Security vulnerabilities must follow `SECURITY.md` and must not be reported in public issues.
