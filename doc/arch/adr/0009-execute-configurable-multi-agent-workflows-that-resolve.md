---
status: proposed
date: 2026-07-24
deciders: [maintainer]
consulted: []
informed: []
---

# Daemon-Owned Configurable Multi-Agent Workflow Executor

## Context and Problem Statement

Feature 001 defines configuration for workflows, roles, and model aliases and
can route a single attempt, but it does not execute multi-stage specification,
review, implementation, validation, and correction loops. Features 004–007
supply supervised providers and a governed tool gateway. Without a workflow
executor, sessions still require the human to advance each stage manually, and
correction loops cannot be bounded or recovered deterministically.

## Decision Drivers

- Keep orchestration ownership in the Rust daemon, not in clients or providers.
- Reuse Feature 001 attempt, routing plan, session control, and configuration
  snapshot contracts.
- Resolve roles to provider-neutral model aliases so Claude, Codex, and Grok
  remain interchangeable behind adapters.
- Bound review-correction loops and never invent success after interruption.
- Prove the primary multi-provider path offline with fakes.
- Avoid VS Code control-room UX, OpenRouter, and free-form DAG scheduling in
  the same change.

## Considered Options

- Daemon-owned sequential workflow executor with bounded correction edges.
- Client-driven stage advancement (VS Code or CLI scripts call each role).
- Provider-native multi-agent frameworks (each CLI runs its own swarm).
- Full DAG / parallel workflow engine in the first increment.

## Decision Outcome

Chosen option: **daemon-owned sequential workflow executor with optional
bounded correction edges**, because it matches the constitution, reuses the
existing orchestrator and session lifecycle, and delivers the product's primary
Claude → Codex → Grok → Codex path without introducing a second permission or
history model.

The executor:

1. validates versioned workflow graphs at configuration time;
2. pins the session configuration snapshot when a run starts;
3. advances sequential steps and optional `on_findings` correction targets
   under `max_iterations`;
4. emits explainable routing plans (`SelectedRule::Workflow`) before each
   dispatch;
5. honors pause, resume, cancel, and redirect on the session lifecycle;
6. recovers phase and active step from durable events after restart;
7. routes tools only through the Feature 007 gateway.

### Consequences

- Good: multi-stage work becomes durable, auditable, and recoverable.
- Good: provider swaps stay configuration-local via roles and model aliases.
- Good: offline acceptance can prove the primary multi-provider path.
- Bad: first increment is sequential + single correction edge, not a general
  DAG or parallel fan-out engine.
- Bad: findings classification must stay explicit and testable; ambiguous
  natural-language "looks fine" signals are out of scope for auto-advance.
