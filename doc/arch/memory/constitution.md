# Multi-Agent Development Workbench Constitution

This constitution defines the non-negotiable design and delivery constraints
for the Workbench.

## Principles

### 1. Specification before implementation

`doc/arch/` is the product source of truth. Every behavior change starts with a
tracked change request and an active Speckit feature. Work follows the phase
reported by `speckit next`; implementation cannot begin early, and a failing
`speckit validate` blocks delivery.

### 2. Provider-independent core

Workflows target stable roles and capabilities, never hard-coded vendors.
Provider-specific authentication, process control, and protocol behavior stay
inside replaceable adapters. Adding or removing a compatible model should
normally require configuration, not orchestration changes.

### 3. Human visibility and control

Every dispatch exposes its destination, context, tools, permissions, risk, and
progress. Users can pause, cancel, resume, comment on, or redirect active work.
The system never broadcasts prompts implicitly, and uncertainty or sensitive
actions require explicit human approval.

### 4. Local-first security and least privilege

Credentials remain in provider-owned stores or the operating-system keychain.
The daemon grants tools by role and session, defaults integrations to read-only,
and records approvals and tool activity. Repository configuration cannot widen
the user's global security policy.

### 5. Durable, reproducible, and testable execution

Sessions use an append-only event history and retain redacted configuration
snapshots, routing decisions, artifacts, and outcomes. Protocols and schemas are
versioned. Automated tests use fake adapters by default and must not consume
paid model quotas.

### 6. Thin, portable clients

The Rust daemon owns orchestration, policy, storage, provider lifecycle, and MCP
management. VS Code, terminal, ACP, and future editor integrations remain
presentation clients over documented protocols so the product can migrate
between interfaces without rebuilding its core.

## Governance

Changes to these principles require an accepted Architecture Decision Record
with named deciders. Product behavior and contract changes require the tracked
change process, an active Speckit feature, review, and green validation.
Trivial wording or formatting corrections may skip a feature but must preserve
meaning. Specifications, code, comments, commits, and contribution artifacts
are written in English.
