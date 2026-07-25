---
id: 019f9720-a1b2-7c3d-8e4f-0100a0be0001
number: 010
slug: add-an-openrouter-api-provider-with-cost-controls
status: implemented
created_at: 2026-07-24T23:30:00.000000Z
---
# Feature Specification: OpenRouter API Provider and Cost Controls

Feature: 010-add-an-openrouter-api-provider-with-cost-controls
Created: 2026-07-24
Related issue: #16

## Objective

Add a production API-provider adapter for OpenRouter that resolves credentials
only through opaque OS keychain references, streams OpenAI-compatible Chat
Completions responses into the common provider port, and enforces configurable
USD budget ceilings before paid dispatch. Default automated suites must remain
offline, deterministic, and free of paid network calls.

## Scope

This feature includes:

- `type: api` provider composition for OpenRouter Chat Completions streaming;
- credential resolution via `platform:` / `keychain:` / `secret-service:`
  handles only (never plaintext secrets in config, locks, events, or logs);
- privacy flags already required on API providers (`zero_data_retention`,
  `data_collection`);
- session and optional attempt USD budget policy with fail-closed enforcement
  before HTTP dispatch;
- durable redacted usage and spend accounting sufficient to refuse further
  paid attempts when the session budget is exhausted;
- offline fake HTTP transport and acceptance harness;
- optional live smoke tests marked `#[ignore]` that never run by default.

This feature excludes:

- the Workbench ACP server and terminal client (issue #17);
- embedding OpenRouter keys in repositories or session databases;
- automatic budget top-up or payment methods;
- Responses API exclusive-only paths when Chat Completions covers MVP needs;
- free-form multi-provider spend aggregation across workspaces;
- changing subscription-cli Grok/Claude/Codex billing models.

## User Stories

- As an operator, I want to route selected roles to OpenRouter models with an
  API key stored only in the OS keychain.
- As a budget owner, I want Workbench to refuse paid OpenRouter calls when the
  configured session budget would be exceeded.
- As a developer, I want offline fakes that prove streaming, missing
  credential, over-budget denial, cancellation, and secret containment without
  spending credits.
- As a security-conscious user, I want missing credentials and over-budget
  conditions to fail closed before any external request is claimed.

## Functional Requirements

1. **FR-010-001:** A provider with `type: api` MUST declare `credential_ref`
   and `privacy`. Workbench MUST NOT accept plaintext API keys in
   configuration, locks, protocol payloads, events, telemetry, or logs.
2. **FR-010-002:** `credential_ref` MUST match
   `^(platform|keychain|secret-service):[a-zA-Z0-9._/-]+$`. Resolution MUST
   use the OS credential store (or an injected offline fake store in tests).
   Missing or empty secrets MUST leave the provider unavailable and MUST NOT
   start HTTP dispatch.
3. **FR-010-003:** The OpenRouter adapter protocol identity MUST be
   `openrouter-chat-completions/1`. Runtime models are configuration aliases
   that map to OpenRouter model ids without adapter code changes for new
   compatible models.
4. **FR-010-004:** Default endpoint identity MUST be
   `https://openrouter.ai/api/v1`. Offline tests MUST inject a fake HTTP
   transport or loopback-only endpoint and MUST NOT dial the public network
   by default.
5. **FR-010-005:** Prompt streaming MUST use OpenAI-compatible Chat
   Completions with `stream: true`, Bearer authorization from the resolved
   secret, and bounded frame/body limits (8 MiB encoded ceiling). Malformed
   SSE, oversized bodies, and non-UTF-8 payloads MUST fail closed with a
   redacted error.
6. **FR-010-006:** Partial assistant deltas MUST become ordered content
   events without duplicating final text. A definite terminal completion MUST
   emit exactly one `Completed` outcome. Transport loss after dispatch starts
   and before a definite terminal result MUST yield `outcome_unknown` without
   automatic retry.
7. **FR-010-007:** Configuration MUST support optional cost policy under
   `policies.cost` with at least `max_session_usd_micros` (integer
   micro-USD). When any `type: api` provider is present, `policies.cost` MUST
   be present and `max_session_usd_micros` MUST be greater than zero, or
   validation MUST fail closed.
8. **FR-010-008:** Optional `max_attempt_usd_micros` MAY further cap a single
   attempt estimate. Enforcement MUST run before HTTP dispatch: if recorded
   session spend plus the attempt estimate would exceed the session budget,
   or the estimate exceeds the attempt ceiling, the attempt MUST be denied
   with a stable policy/budget error and MUST NOT start the HTTP call.
9. **FR-010-009:** Successful completions MUST record redacted usage
   (token counts and spend micros when advertised by the provider or
   estimated from a local rate table) into an in-process session cost ledger
   consulted by subsequent budget checks. Spend figures MUST never include
   the API key or raw request bodies.
10. **FR-010-010:** Paid OpenRouter inference MUST set `material_cost: true`
    and `effect_class: paid-inference` semantics already used by the
    orchestrator, so approval and no-auto-retry rules continue to apply.
11. **FR-010-011:** Cancellation MUST stop the active stream within the
    existing public five-second cancellation budget (provider share plus
    daemon finalization reserve). Unconfirmed cancellation after dispatch
    MUST surface `outcome_unknown` rather than success.
12. **FR-010-012:** Privacy settings MUST be retained on the provider model.
    When `data_collection: deny`, the adapter MUST send the OpenRouter
    zero-retention / privacy headers documented for this feature and MUST NOT
    claim retention beyond what the upstream accepts.
13. **FR-010-013:** Default automated tests MUST use only offline fakes, make
    no public network calls, resolve no live keychain secrets unless a
    dedicated platform key-store contract is explicitly selected, and consume
    no OpenRouter credits. Live validation MUST be `#[ignore]` and opt-in.
14. **FR-010-014:** Unique secret markers placed in credential material,
    headers, and response bodies MUST be absent from protocol replies,
    telemetry, locks, SQLite, WAL, exports, and structured logs on success
    and failure paths.
15. **FR-010-015:** Workbench documentation MUST disclose that OpenRouter is
    API-billed per use, credentials stay in the OS store, and budgets fail
    closed without automatic payment recovery.

## Success Criteria

- Offline acceptance harness binds every Gherkin case to executable evidence.
- Missing credential fails before HTTP dispatch.
- Over-budget fails before HTTP dispatch.
- Happy-path fake stream completes once with durable content.
- `speckit validate` and workspace tests remain green for the feature scope.
- `docs/project/STATUS.md` marks Feature 010 delivered and next-ready #17.

## Out of Scope Notes

- Full Responses API multi-tool agent loop beyond Chat Completions streaming.
- Cross-daemon durable cost ledger persistence (MVP ledger is process-local
  for the active daemon; session event payloads may carry redacted spend
  summaries when emitted).
- Grok-derived terminal fork and Workbench ACP server (Feature 011 / #17).

## Observability

Budget denials, missing credentials, and transport failures map to existing
redacted provider failure categories and session events. Spend micros may be
recorded in-process for subsequent budget checks. Secrets, raw request bodies,
and Authorization headers never appear in telemetry labels or structured logs.

