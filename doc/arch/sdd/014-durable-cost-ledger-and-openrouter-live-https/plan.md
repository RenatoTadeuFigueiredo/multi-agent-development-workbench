# Implementation Plan: Durable Cost Ledger and OpenRouter Live HTTPS

## Approach

1. SQLite migration `0002_session_spend` adds `sessions.spend_usd_micros`.
2. `SessionCostLedger` becomes per-session with optional `DurableSpendStore`.
3. Daemon `PathSpendStore` restores/persists through the workstation DB.
4. OpenRouter `OpenRouterTransport::live_https` uses rustls + native roots;
   default composition remains offline fake.
5. Feature 014 acceptance proves durable restore, budget deny after reload, and
   offline-only defaults; live smoke is ignored.

## Acceptance

`feature_014.rs` fingerprints Gherkin and executes offline durable ledger cases.
