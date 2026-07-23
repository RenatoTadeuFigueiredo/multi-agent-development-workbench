# Repository Guidelines

## Project Structure & Module Organization

This repository is currently an empty scaffold. As the project is initialized, keep implementation code under `src/`, automated tests under `tests/`, and static files under `assets/`. Place developer scripts in `scripts/` and architectural or operational documentation in `docs/`.

Organize `src/` by feature or domain rather than by file type. Keep tests close in shape to the code they cover; for example, `src/editor/session.ts` should correspond to `tests/editor/session.test.ts`. Avoid adding generated output, dependency directories, or local editor state to version control.

## Build, Test, and Development Commands

No build system or package manager is configured yet. When adding one, expose a small, predictable command set and document it in `README.md`. Prefer commands such as:

- `make dev` — start the local development environment.
- `make test` — run the complete automated test suite.
- `make lint` — check formatting and static-analysis rules.
- `make build` — produce a release-ready artifact.

Keep these entry points stable even if the underlying tools change.

## Coding Style & Naming Conventions

Follow the formatter and linter native to the selected language, and commit their configuration with the first source files. Use spaces for indentation unless the formatter requires otherwise. Name files and modules consistently: `kebab-case` for general files, `PascalCase` for exported types or components, and `camelCase` for functions and variables. Prefer focused modules and explicit names over abbreviations.

## Testing Guidelines

Every behavior change or bug fix should include an automated test. Name tests after the unit or behavior they verify, using the framework’s standard suffix (for example, `*.test.ts` or `test_*.py`). Cover success paths, expected failures, and meaningful edge cases. Run the full suite before requesting review.

## Commit & Pull Request Guidelines

There is no Git history from which to infer a convention. Until the project establishes one, use concise imperative subjects with a Conventional Commit prefix, such as `feat: add workspace selector` or `fix: preserve unsaved tabs`.

Pull requests should explain the problem, summarize the solution, link the relevant issue, and list verification performed. Include screenshots or recordings for visible UI changes. Keep each pull request narrowly scoped and call out configuration changes, migrations, risks, and follow-up work.

## Repository Identity & Confidentiality

Use `Renato Figueiredo <renato.tadeu.figueiredo@gmail.com>` for project commits. Do not add references to the user's employers, clients, or unrelated organizations in code, documentation, comments, commits, examples, or generated artifacts. Never commit credentials, subscription tokens, API keys, local session databases, or provider authentication state.
