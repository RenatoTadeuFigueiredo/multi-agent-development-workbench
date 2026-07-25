---
status: accepted
date: 2026-07-25
deciders: [maintainer]
consulted: []
informed: []
---

# Workbench ACP Server MVP via agent stdio Bridge

## Context and Problem Statement

Editors and a future terminal client need ACP access to Workbench without
embedding Grok or reimplementing orchestration. Feature 011 must expose a
stable agent surface while keeping the daemon as the control plane.

## Decision Drivers

- One daemon remains the control plane for VS Code, CLI, and ACP clients.
- Offline fakes keep CI zero-quota.
- Defer the Grok-derived pager fork as an explicit residual gap.

## Considered Options

- Ship `workbench agent stdio` as an ACP v1 agent that bridges to the existing
  daemon/fake application path.
- Embed Grok Build as the ACP control plane.
- Defer all ACP exposure until a full terminal fork exists.

## Decision Outcome

Chosen option: **Ship `workbench agent stdio` as an ACP v1 agent bridge**,
because it reuses the existing protocol and offline application path without
embedding Grok.

### Consequences

- Good: one daemon remains the control plane for VS Code, CLI, and ACP clients.
- Good: offline fakes keep CI zero-quota.
- Bad: full terminal UX and editor marketplace packaging remain later work.
