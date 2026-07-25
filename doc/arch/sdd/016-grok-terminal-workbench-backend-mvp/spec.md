---
id: 019f9980-0160-7d4e-9f5a-0160term00001
number: 016
slug: grok-terminal-workbench-backend-mvp
status: implement
created_at: 2026-07-25T15:00:00.000000Z
---
# Feature Specification: Grok Terminal WorkbenchBackend MVP

Feature: 016-grok-terminal-workbench-backend-mvp
Created: 2026-07-25
Related issue: #33

## Objective

Ship a monorepo `WorkbenchBackend` launch surface that the Grok-derived
terminal selects to run `workbench agent stdio`, closing the Known Gap for
terminal/backend integration without forking pager rendering in this repo.

## Scope

Includes:

- `workbench-terminal-backend` crate with validated absolute launch plan;
- argv contract `workbench agent stdio` and workspace cwd;
- compatibility pin constant for the Grok Build fork (may be empty until
  dual-upstream rebase publishes a SHA);
- offline unit + Feature 016 acceptance;
- documentation residual: full Grok pager fork/rebase remains in the
  `grok-build` repository, not this monorepo.

Excludes:

- Embedding the Grok pager source tree;
- PTY widget/rendering changes;
- Provider credential handling in the terminal.

## Functional Requirements

1. **FR-016-001:** WorkbenchBackend MUST plan `agent stdio` with absolute paths.
2. **FR-016-002:** Relative or parent-traversal executables MUST fail closed.
3. **FR-016-003:** Compatibility pin constant MUST exist for fork sync.
4. **FR-016-004:** STATUS Known Gaps MUST remove or reclassify #33 with residual.

## Success Criteria

- Offline Feature 016 green.
- Known Gaps empty or only truly out-of-tree residuals.
