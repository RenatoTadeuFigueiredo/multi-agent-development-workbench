---
id: 019f9940-2b3c-7d4e-9f5a-0140c05a0001
number: 014
slug: durable-cost-ledger-and-openrouter-live-https
status: implement
created_at: 2026-07-25T13:00:00.000000Z
---
# Feature Specification: Durable Cost Ledger and OpenRouter Live HTTPS

Feature: 014-durable-cost-ledger-and-openrouter-live-https
Created: 2026-07-25
Related issue: #31

## Objective

Persist redacted per-session paid-inference spend across daemon restarts and
compose an opt-in OpenRouter live HTTPS Chat Completions client while keeping
default CI offline and quota-free.

## Scope

Includes:

- durable redacted `spend_usd_micros` per session in encrypted SQLite;
- `SessionCostLedger` restore/persist via `DurableSpendStore`;
- fail-closed budget checks that honor prior spend after restart;
- live HTTPS OpenRouter transport (`rustls`) available when explicitly enabled;
- offline default composition and `#[ignore]` live smoke only.

Excludes:

- multi-tool Responses API agent loop;
- ACP attach (#29);
- provider-native write tools (#32).

## Functional Requirements

1. **FR-014-001:** Session spend micros MUST persist without API keys or raw
   request/response bodies.
2. **FR-014-002:** After restore, pre-dispatch budget checks MUST deny when
   prior spend plus estimate would exceed `policies.cost`.
3. **FR-014-003:** Default transport MUST remain offline fake for CI.
4. **FR-014-004:** Live HTTPS MUST use rustls with native roots when enabled.
5. **FR-014-005:** Live network smoke MUST be `#[ignore]` opt-in only.

## Success Criteria

- Offline acceptance Feature 014 green.
- STATUS Known Gaps removes or narrows #31.
