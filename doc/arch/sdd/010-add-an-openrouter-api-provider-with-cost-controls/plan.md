# Implementation Plan: OpenRouter API Provider and Cost Controls

## Overview

Ship Feature 010 as a bounded `workbench-openrouter` crate that implements the
shared `ProviderAdapter` port over OpenAI-compatible Chat Completions, resolves
credentials only through opaque OS secret handles, and enforces configurable
USD budgets before dispatch. Compose the adapter in `workbench-daemon` the same
way Features 004–006 compose supervised CLI adapters, with offline fakes as the
default path.

## Technical Approach

### Configuration

- Keep `type: api` + required `credential_ref` + `privacy`.
- Add optional `base_url` on providers (defaults to
  `https://openrouter.ai/api/v1`; tests may use `fake://openrouter` or inject a
  transport without changing production defaults).
- Extend `policies` with optional `cost`:
  - `max_session_usd_micros: u64` (required when cost present)
  - optional `max_attempt_usd_micros: u64`
- Validate: when any provider has `type: api`, `policies.cost` must be present
  and `max_session_usd_micros > 0`.
- Update `workbench-configuration.schema.json` and generated fixtures.

### Crate `workbench-openrouter`

| Module | Responsibility |
|---|---|
| `adapter` | `ProviderAdapter` implementation |
| `transport` | offline fake + real HTTPS Chat Completions stream |
| `budget` | estimate + session ledger + pre-dispatch gate |
| `credential` | resolve `credential_ref` via `SecretSource` port |
| `protocol` | SSE / JSON delta parsing, usage extraction |
| `error` | redacted failure kinds |

Protocol constant: `openrouter-chat-completions/1`.

### Credential resolution

```text
credential_ref → SecretSource::resolve(ref) → Zeroizing<String>
```

- Production: platform keyring entry under the Workbench service id.
- Tests: `MemorySecretSource` map.
- Missing/empty → `AuthenticationStatus::Unavailable` and definite pre-dispatch
  failure.

Never log, persist, or include secret bytes in events.

### Budget enforcement

1. Load session spend micros from `SessionCostLedger`.
2. Estimate attempt cost (conservative local rate table × max tokens, or fixed
   attempt ceiling when configured).
3. If `spend + estimate > max_session` or `estimate > max_attempt` →
   `PolicyDenied` / budget error, no HTTP.
4. On successful terminal usage, add spend to the ledger.

### Daemon composition

- Extend `ProviderRuntime` / bootstrap to connect `type: api` providers.
- Inject shared `SessionCostLedger` and secret source.
- Register capabilities: streaming, cancellation; no tool-calling claim in MVP.
- Paid inference remains `material_cost: true` at orchestrator dispatch.

### Acceptance

`workbench-testkit/tests/feature_010.rs`:

- fingerprint every Gherkin case;
- prove offline happy path, missing credential, over-budget, frame bounds,
  malformed SSE, cancellation, secret containment, zero network/quota defaults;
- live test file `live_openrouter.rs` with `#[ignore]`.

## File / crate impact

| Area | Change |
|---|---|
| `workbench-openrouter` | new crate |
| `workbench-config` | cost policy, base_url, validation |
| `workbench-daemon` | compose API adapter + ledger |
| `workbench-testkit` | feature_010 + optional fake helpers |
| `Cargo.toml` workspace | member + deps |
| `doc/arch` | spec, plan, tasks, Gherkin, CUE, ADR |
| `docs/project/STATUS.md` | delivered 010, next #17 |
| `Makefile` | `test-openrouter` / acceptance entry |

## Dependencies

- Features 001–009 on `main`.
- Issue #16.
- No dependency on #17 for acceptance.

## Risks

- Public HTTPS client expands supply chain; pin minimal HTTP stack or reuse
  tokio TCP + rustls carefully. Prefer a tiny custom client for Chat
  Completions streaming if workspace lacks reqwest, matching MCP HTTP style
  for loopback/fake-first and live-only for ignored tests.
- Budget estimates may under/over-count vs OpenRouter billing; fail closed on
  ceiling breach using conservative estimates and recorded usage when present.
- Keychain availability varies by host; missing secret is an operator fix, not
  a silent fallback to env vars.

## Rollback

Revert the feature branch/merge. API providers without the adapter remain
configuration-only and non-dispatchable.
