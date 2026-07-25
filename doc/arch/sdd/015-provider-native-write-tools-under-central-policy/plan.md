# Implementation Plan: Provider-Native Write Tools Under Central Policy

## Approach

1. Add `policies.provider_native_writes` (mode + allowlist), default disabled.
2. Thread allow decision into Claude/Codex launch profiles.
3. Expand protocol allowlists only when the profile enables native writes.
4. Offline unit tests for grant/deny; Feature 015 acceptance inventory.

## Acceptance

`feature_015.rs` fingerprints Gherkin and proves fail-closed defaults plus
explicit allowlist enablement.
