# Publication Runbook — Specification Phase

The repository currently publishes specifications and architecture only; it
does not produce a deployable Workbench binary. This runbook governs a
documentation release from a clean checkout and must be replaced by an
artifact deployment runbook before the first binary release.

## Purpose

Publish a reviewed, reproducible Speckit corpus without implying that product
code or provider integrations are available.

## Trigger

Run this procedure when:

- A reviewed specification change is approved for `main`.
- A documentation tag or public architecture snapshot is requested.

## Preconditions

- The change is linked to its tracked issue and has completed human review.
- The working tree is clean on the intended commit.
- `git`, `make`, Rust 1.95.0, and the pinned Speckit revision are available.
- No product binary or container is included in the release.

## Steps

1. Fetch the remote and check out the exact reviewed commit.
2. Confirm `git status --short` is empty.
3. Run `speckit status` and verify the reported feature and phase.
4. Run `speckit validate`; require zero findings.
5. Run `speckit spec score` and record the project health in the review.
6. Run `make check`; unbound Gherkin scenarios are expected only until their
   feature reaches implementation.
7. Verify Markdown links, Mermaid parsing, and the English/pt-BR README
   structure.
8. Merge through the approved pull request. Create a documentation tag only
   when the release request explicitly requires one.

## Verification

- `speckit validate` reports zero findings.
- `make check` exits successfully.
- The published commit equals the reviewed commit.
- The README still states that implementation has not begun.

## Rollback

If any step fails or verification does not pass:

1. Stop publication when any verification step fails.
2. If a bad documentation commit reached `main`, create a reverting change
   through the normal review flow; do not rewrite shared history.
3. Re-run `speckit validate` and `make check` on the revert.
4. Record the failure on the tracked issue before republishing.
