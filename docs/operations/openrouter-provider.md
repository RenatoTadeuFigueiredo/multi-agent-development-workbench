# OpenRouter Provider Operations

Feature 010 adds a supervised OpenRouter Chat Completions adapter. It does not
replace subscription routes for Claude, Codex, or Grok.

## Configuration

```yaml
providers:
  openrouter:
    type: api
    credential_ref: platform:openrouter
    privacy:
      zero_data_retention: true
      data_collection: deny

models:
  api-implementer:
    provider: openrouter
    runtime_model: x-ai/grok-code-fast-1

policies:
  cost:
    max_session_usd_micros: 5000000
    max_attempt_usd_micros: 500000
```

Store the API key in the OS keychain under the Workbench service using the
opaque handle named by `credential_ref`. Never commit the key.

## Behavior

- Protocol identity: `openrouter-chat-completions/1`.
- Default base URL: `https://openrouter.ai/api/v1`.
- Missing or empty secrets fail closed before HTTP.
- Session and optional attempt budgets fail closed before HTTP.
- Default automated tests use offline fakes only.
- Live smoke tests are `#[ignore]` and opt-in.

## Billing

OpenRouter is billed per use against the operator's OpenRouter credits.
Workbench budgets are local fail-closed ceilings; they are not a payment
method and do not top up credits.
