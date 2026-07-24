# Tasks: Add a Versioned Session List Command to the Local Workbench

## Task Breakdown

- [x] T001 Define the versioned `session.list` protocol contract, metadata-only
  summary, bounded exclusive cursor, AsyncAPI fixture, domain semantics, CUE
  value object, feature specification, and ADR.
- [x] T002 Implement deterministic persistent-session pagination in storage and
  expose the daemon-scoped read command through the protocol and daemon layers.
- [x] T003 Add CLI parsing/mapping for `workbench session list`, including
  parameter validation and command-construction coverage.
- [x] T004 Add VS Code New Session and Select Session commands, preserving
  attach-by-ID and restricting discovery to the configured workspace endpoint.
- [x] T005 Add deterministic Rust/TypeScript coverage and run contract, CLI,
  extension compile, and extension test validation.

## Dependencies

The feature depends only on the existing persistent-session store and the
versioned local daemon protocol. The VS Code workflow requires an unlocked
local Workbench daemon for manual use; all automated tests use offline fakes.
