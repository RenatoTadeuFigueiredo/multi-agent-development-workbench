# Contributing

Thank you for helping improve the Multi-Agent Development Workbench.

Participation is governed by `CODE_OF_CONDUCT.md`.

## Before Starting

1. Run `make context` to reconstruct the durable project state.
2. Read `AGENTS.md`, `docs/project/STATUS.md`, and the English `README.md`.
3. Search existing issues before proposing a change.
4. Open or reference an issue that explains the problem and expected outcome.
5. Create a focused branch from `main`.

Product features and behavior changes must follow the active Speckit feature
and its current phase. Do not implement product code before the workflow
reaches `implement`, and do not bypass a failing `speckit validate`.

Terminal UI work follows the separate branch policy documented in
`docs/architecture/grok-build-terminal-integration.md`. In the Grok Build fork,
`main` is a fast-forward-only upstream mirror. Create product branches from
`workbench` and target pull requests back to `workbench`; never add Workbench
commits to the fork's `main`.

Routing, configuration, and provider work must follow
`docs/architecture/configuration-routing-and-providers.md`.

## Development Standards

- Keep application logic and first-party binaries in Rust.
- Keep `extensions/vscode/` as a thin TypeScript client; orchestration, provider, credential, and policy logic belongs in Rust.
- Reference stable roles and model aliases from workflows. Do not embed vendor
  or model selection in the orchestration core.
- Keep provider-specific behavior behind the shared capability contract and
  register adapters without provider-name conditionals in domain code.
- Keep the Grok-derived terminal client presentation-only. Multi-provider
  workflows, MCP lifecycle, credentials, and policy decisions belong in
  `workbench daemon`.
- Reuse upstream pager actions, effects, widgets, rendering, and tests. Add a
  narrow external ACP backend instead of reimplementing or broadly modifying
  the TUI.
- Make surgical changes and avoid unrelated refactors.
- Run `make check` before requesting review. It verifies Rust formatting,
  Clippy, contract drift, workspace tests, all committed feature acceptance
  harnesses, SLOs, and the Speckit gates.
- Use `make test-platform` only with an expendable, unlocked OS credential
  store; the default suite uses the in-memory key store.
- Add tests for behavior changes, failures, and meaningful edge cases.
- Test configuration precedence, schema migration, capability preflight,
  routing explanations, fallbacks, and provider removal when those behaviors
  change.
- Keep the English and PT-BR README versions aligned when changing shared content.
- Never commit credentials, provider sessions, API keys, or local databases.

See the
[feature 001 quickstart](doc/arch/sdd/001-build-the-workbench-orchestration-kernel-foundation-as-a/quickstart.md)
for the executable fake-provider flow.

## Pull Requests

Use a concise Conventional Commit subject. Pull requests should:

- Explain the problem and chosen solution.
- Link the related issue or specification.
- List validation performed.
- Describe security, compatibility, migration, and rollback considerations.
- Include screenshots or recordings for visible interface changes.
- Update `docs/project/STATUS.md` when delivered capabilities, known gaps,
  dependencies, or roadmap priority change.
- Remain narrowly scoped and ready for review.

Grok Build upstream-sync pull requests must additionally:

- identify the previous and proposed upstream commits;
- include a `git range-diff` of the downstream patch stack;
- preserve the original `grok` backend and behavior;
- report upstream pager, Workbench ACP contract, PTY, and snapshot results; and
- remain manual-review only, even when the rebase and tests are clean.

Security vulnerabilities must follow `SECURITY.md` and must not be reported in public issues.
