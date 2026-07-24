# Observability Strategy

Workbench telemetry explains routing, permissions, provider lifecycle, session
controls, and recovery without exposing prompt content or credentials. Keep
this document aligned with feature-level `## Observability` sections.

## Signals

- **Metrics** report bounded health, throughput, latency, and saturation.
- **Logs** carry structured lifecycle, decision, and failure events.
- **Traces** follow one prompt or control across daemon boundaries.

## Metrics

Initial instruments:

| Instrument | Type | Unit | Purpose |
|---|---|---|---|
| `workbench.session.active` | gauge | sessions | Active sessions by state |
| `workbench.route.decision` | counter | decisions | Routing rule and outcome |
| `workbench.provider.duration` | histogram | milliseconds | Adapter operation latency |
| `workbench.policy.decision` | counter | decisions | Allowed, denied, or approval-required actions |
| `workbench.control.duration` | histogram | milliseconds | Pause, resume, redirect, and cancel latency |
| `workbench.replay.lag` | histogram | events | Events returned after a reconnect cursor |
| `workbench.storage.decision` | counter | decisions | Encryption, key-store, retention, export, and deletion outcomes |

## Logs

Records include timestamp, severity, component, event name, outcome, session
correlation identifier, and error category. Routing plans, retry decisions,
uncertain outcomes, and key lifecycle actions log metadata only. Prompt bodies,
decrypted payloads, ciphertext, nonces, model output, tool payloads,
credentials, provider session tokens, and environment values are excluded.

## Tracing

One root span represents a user input or session control. Child spans cover
configuration resolution, routing, policy evaluation, event append, adapter
preflight, provider execution, and replay. Session and correlation identifiers
are span attributes, never metric labels.

## Cardinality

Never use user, request, session, email, path, model name, or UUID values as
metric labels.

| Label | Bounded Value Set | Signal |
|-------|-------------------|--------|
| session_state | ready, running, pausing, paused, awaiting_clarification, awaiting_approval, cancel_requested, outcome_unknown, completed, failed, cancelled, abandoned, deleting | session gauge |
| route_rule | explicit, workflow, resolver, coordinator, clarification | routing counter |
| outcome | success, denied, failed, cancelled, abandoned, timeout, outcome_unknown, client_lagged | lifecycle instruments |
| adapter_kind | subscription-cli, api, acp, fake | provider histogram |
| control | pause, resume, redirect, cancel | control histogram |
| policy_result | allowed, approval-required, denied | policy counter |
| storage_action | encrypt, decrypt, rotate, export, delete | storage counter |

## OTLP Conventions

Telemetry export is off by default. When enabled, the daemon exports OTLP from
the application boundary and sets
`service.name = "multi-agent-development-workbench"`. Export failures never
block session execution and are reported through a bounded health signal.
