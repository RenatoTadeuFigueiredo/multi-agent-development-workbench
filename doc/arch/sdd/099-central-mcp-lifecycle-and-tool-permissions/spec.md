---
id: 019f95fd-ed53-7ca0-ab38-1cf0eb49287e
number: 099
slug: central-mcp-lifecycle-and-tool-permissions
status: draft
created_at: 2026-07-24T21:17:54.38757Z
archived: true
---
# Archived Draft: Central MCP Lifecycle and Tool Permissions

Feature: 099-central-mcp-lifecycle-and-tool-permissions
Created: 2026-07-24

## Archive Note

This entry is an archived accidental draft created when `speckit specify`
partially ran before branch creation failed, then was renumbered away from
`007`. It is retained only for Speckit archive permanence.

**Authoritative specification:**
`doc/arch/sdd/007-central-mcp-lifecycle-and-tool-permissions/spec.md`

Do not implement against this archived draft. Do not restore it unless
recovering Speckit metadata for forensics.

## User Stories

- As an operator I want this archived draft retained so Speckit history remains
  recoverable without implementing a duplicate feature.

## Functional Requirements

1. **FR-099-001:** This archived draft MUST NOT be selected as an active
   implementation target. All product requirements live under Feature 007.

## Security Requirements

Not applicable — this archived draft defines no product behavior, data paths,
credentials, or runtime surface; Feature 007 owns the security requirements.

## Acceptance Scenarios

1. **Archive inert:** Given Feature 099 is archived, when Speckit status is
   consulted for product work, then Feature 007 remains the active MCP gateway
   specification.

## Observability

Not applicable — this archived draft emits no metrics, logs, or traces.
Feature 007 owns observability for the MCP gateway.
