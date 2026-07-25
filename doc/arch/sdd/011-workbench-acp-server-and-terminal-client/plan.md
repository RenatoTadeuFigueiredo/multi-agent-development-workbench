# Implementation Plan: Workbench ACP Server MVP

## Approach

Add `workbench-acp-server` as a thin crate that:

1. reads NDJSON JSON-RPC from stdin and writes to stdout;
2. maps ACP agent methods onto `workbench-protocol` client commands against a
   running or embedded offline daemon harness;
3. normalizes daemon events into ACP `session/update` notifications.

Wire `workbench agent stdio` in `workbench-cli` to that crate.

## Acceptance

`feature_011.rs` fingerprints Gherkin cases and proves initialize, session new,
prompt stream, cancel, frame bounds, and offline-only defaults.

## Residual gap

Grok-derived terminal pager fork remains deferred; document in STATUS Known Gaps.
