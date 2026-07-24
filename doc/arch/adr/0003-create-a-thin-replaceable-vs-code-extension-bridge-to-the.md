---
status: proposed
date: 2026-07-24
deciders: [workbench-maintainers]
consulted: []
informed: []
---

# Create A Thin Replaceable Vs Code Extension Bridge To The

## Context and Problem Statement

The first kernel release already exposes a versioned local daemon protocol.
Users need an editor surface for observing and controlling sessions, but the
surface must remain replaceable and must not duplicate orchestration policy.

## Decision Drivers

- Preserve the Rust daemon as the only orchestration and security boundary.
- Provide live, inspectable session work in VS Code without provider coupling.

## Considered Options

- A thin TypeScript extension using the existing local protocol.
- A second orchestration implementation embedded in the extension.

## Decision Outcome

Chosen option: "a thin protocol-only TypeScript extension", because it adds the
required editor workflow while keeping provider, policy, persistence, and
future editor migrations outside the presentation layer.

### Consequences

- Good: VS Code can be replaced without changing the daemon contracts.
- Good: Credentials and transcripts remain governed by the daemon.
- Bad: The extension depends on protocol compatibility and requires a separate
  TypeScript test/package toolchain.
