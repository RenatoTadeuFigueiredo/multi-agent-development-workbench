---
status: accepted
date: 2026-07-25
deciders: [maintainer]
---

# Workbench ACP Server MVP via agent stdio Bridge

## Context

Editors and a future terminal client need ACP access to Workbench without
embedding Grok or reimplementing orchestration.

## Decision

Ship `workbench agent stdio` as an ACP v1 agent that bridges to the existing
daemon protocol. Defer the Grok-derived pager fork as an explicit residual gap.

## Consequences

- Good: one daemon remains the control plane for VS Code, CLI, and ACP clients.
- Good: offline fakes keep CI zero-quota.
- Bad: full terminal UX and editor marketplace packaging remain later work.
