---
status: accepted
date: 2026-07-24
deciders: [workbench-maintainers]
consulted: []
informed: []
---

# Add a Versioned Session List Command to the Local Workbench

## Context and Problem Statement

The existing local protocol permits attaching only when a user already knows a
session ID. That is adequate for scripts but forces VS Code users to obtain an
ID outside the editor. Session discovery must remain local, bounded, and safe:
it cannot expose encrypted transcript content or create a global registry of
sessions across independent daemon endpoints.

## Decision Drivers

- Preserve the daemon as the owner of persistence, authorization, and protocol
  compatibility.
- Make selection practical for the CLI and VS Code without disclosing session
  content or configuration.
- Keep result size bounded and pagination deterministic for offline clients.
- Scope discovery to one same-user local endpoint and one configured workspace.
- Prevent an ambiguous legacy global database from being silently assigned to
  the wrong workspace or replaced by an apparently empty workspace database.

## Considered Options

- Add a daemon-scoped, metadata-only, paginated `session.list` command to
  `workbench/1`.
- Have clients scan the encrypted store directly or maintain their own local
  session history.
- Add a global session index shared by all local daemon endpoints.
- Keep the legacy global runtime layout and rely only on explicit endpoint
  configuration for workspace selection.
- Automatically copy the legacy global SQLite database into the first
  workspace that starts.

## Decision Outcome

Chosen option: "a daemon-scoped metadata-only `session.list` command", because
the daemon can enforce the existing local security boundary, paginate indexed
metadata consistently, and give all thin clients one stable versioned surface.
Default runtime state and sockets are additionally keyed by a stable identifier
derived from the canonical workspace path. Because the legacy global database
contains no workspace identity and its key namespace is bound to its canonical
location, startup fails closed when that database exists but the selected
workspace database does not. It never chooses a workspace or copies the
database automatically.

### Consequences

- Good: CLI and VS Code can discover sessions without embedding storage logic
  or reading transcript content.
- Good: An exclusive cursor and bounded limit make result size and page
  progression predictable.
- Good: The workspace-selected endpoint prevents accidental cross-workspace
  discovery.
- Bad: Older daemons do not support the new command, so clients must retain
  attach-by-ID and present an actionable compatibility error.
- Bad: The list is intentionally a summary view; clients needing events must
  explicitly attach after selection.
- Bad: A workstation with legacy global state cannot start a fresh
  workspace-scoped daemon until the operator follows the migration runbook.
- Bad: This release can export legacy sessions but does not yet provide an
  explicit session importer, so operators needing uninterrupted access must
  remain on the previous release.

### Migration and Rollback

The supported transition is explicit: use the previous release at the legacy
path to create encrypted age exports for every retained session. Never copy the
SQLite file into a workspace directory. Until a supported importer exists,
retain both the previous release and the legacy database at its original path
for continued access, or archive the database after verified export to accept a
fresh workspace with no imported history.

Rollback means stopping workspace-scoped daemons and restoring the previous
release with the untouched legacy database at its original canonical path.
Workspace databases created after the transition remain isolated and are not
merged back into legacy global state.
