---
id: 019f9921-0f4d-7cc2-818d-44ac7810f257
number: 013
slug: compose-tls-https-client-for-non-loopback-mcp-endpoints-so
status: implemented
created_at: 2026-07-25T11:55:08.493323Z
---
# Feature Specification: Compose TLS HTTPS Client for Non-Loopback MCP Endpoints

Feature: 013-compose-tls-https-client-for-non-loopback-mcp-endpoints-so
Created: 2026-07-25
Related issue: #30

## Objective

Compose a supply-chain-conscious TLS stack for the central MCP HTTP client so
non-loopback `https://` MCP endpoints can be invoked through the daemon gateway
with pinned endpoint identity, certificate verification, unpinned-redirect
rejection, and existing response size / redaction bounds. Remove the temporary
fail-closed stub that marks non-loopback HTTPS unavailable when no TLS client
is present.

## Scope

Includes:

- TLS-backed HTTPS transport in `workbench-mcp` for all `https://` MCP URLs
  (non-loopback production path and loopback HTTPS when configured);
- system/web trust roots for production verification;
- offline TLS test fixture (local server + custom trust roots) proving the
  HTTPS path without public network access;
- preservation of cleartext loopback `http://` path and existing fake offline
  client;
- optional `#[ignore]` live HTTPS smoke against a public endpoint;
- daemon composition that enables the live HTTP/TLS client when not offline;
- Speckit corpus, acceptance binding, and STATUS Known Gaps update.

Excludes:

- changing MCP allowlist, approval, or pin semantics beyond TLS availability;
- OpenRouter HTTPS client (issue #31) unless a shared helper is strictly
  necessary and remains MCP-scoped;
- provider-native write tools (issue #32);
- live package registry discovery as a default suite dependency;
- custom operator-managed CA configuration UI (may use injected roots in tests
  only).

## User Stories

- As an operator, I want remote MCP servers over HTTPS so shared tools can run
  outside loopback without disabling gateway governance.
- As a security reviewer, I want non-loopback cleartext HTTP and unpinned
  redirects to keep failing closed, and TLS handshake failures to never leak
  credentials or tool payloads into logs.
- As a developer, I want offline TLS fixtures that prove success and fail-closed
  paths without dialing the public internet by default.

## Functional Requirements

1. **FR-013-001:** When an MCP HTTP server URL uses scheme `https`, the gateway
   HTTP client MUST perform a TLS handshake before sending the MCP request body.
2. **FR-013-002:** Non-loopback hosts MUST use `https` (already enforced by
   endpoint parse). The client MUST NOT return the temporary "TLS not composed"
   unavailability once a TLS stack is present for HTTPS.
3. **FR-013-003:** Production TLS verification MUST use the platform native
   certificate store (via a license-compatible pure-Rust client stack).
   Certificate name verification MUST bind to the pinned host identity.
4. **FR-013-004:** Loopback `http://` cleartext behavior MUST remain available
   for offline and local development paths.
5. **FR-013-005:** Unpinned redirects MUST remain rejected. Response size
   ceilings (default 8 MiB, tighter per-server) MUST still apply to HTTPS bodies.
6. **FR-013-006:** Default automated tests MUST remain offline: either in-process
   fakes or a local TLS fixture. Public network TLS MUST be `#[ignore]` / opt-in.
7. **FR-013-007:** TLS and transport failures MUST map to redacted gateway error
   categories without embedding certificate PEM, private keys, Authorization
   headers, tool arguments, or raw response bodies in public messages or audit.
8. **FR-013-008:** New dependencies MUST be workspace-pinned, license-compatible
   with `deny.toml`, and MUST NOT introduce `reqwest` or `hyper` into
   `workbench-mcp` (Feature 007 supply-chain contract).
9. **FR-013-009:** Daemon composition MUST enable the non-fake HTTP/TLS client
   for production attach so non-loopback HTTPS servers are reachable after pin
   verification.

## Security Requirements

- **Data sensitivity/classification.** MCP tool arguments and results may carry
  confidential source, credentials, or infrastructure inventory. TLS protects
  in-transit confidentiality for non-loopback hops. Durable Workbench surfaces
  still store only redacted lifecycle metadata.
- **Authentication/authorization.** No new multi-tenant network API. Optional
  MCP header secret-handles remain resolved at call time and never appear in
  locks, events, or logs. TLS is transport authentication of the server via
  certificate chain verification, not operator login.
- **Input validation.** Absolute HTTP(S) URLs, redirect locations, and response
  bodies remain untrusted. Allocation stays bounded by the existing 8 MiB
  ceiling. Certificate material used only for verification is never logged.
- **Cryptography in transit/at rest.** Non-loopback MCP HTTP uses TLS 1.2+ via
  the composed stack. Session envelopes and OS credential stores are unchanged.
- **Logging/audit.** Audit continues to record server id, transport class,
  lifecycle phase, correlation, and stable outcomes. No PEM, SNI secrets, or
  raw frames.
- **Error-handling information exposure.** Handshake, timeout, and redirect
  failures map to existing redacted categories (`TransportFailed`,
  `RedirectRejected`, `Timeout`, `Unavailable`) without OS or library strings
  that embed paths or hosts beyond stable categories.

## Acceptance Scenarios

1. **Non-loopback HTTPS success (offline TLS fixture):** Given a pinned
   `https://` MCP identity and a local TLS fixture trusted via test roots, when
   the client invokes a tool, then the call succeeds through TLS and returns a
   redacted success outcome.
2. **Cleartext non-loopback still rejected:** Given `http://example.com/...`,
   when the endpoint is parsed or invoked, then configuration remains invalid
   or the call fails closed without cleartext egress.
3. **Unpinned redirect fails closed:** Given an HTTPS response that redirects to
   another host, when the client handles the response, then the call fails with
   redirect rejection and is not reported as success.
4. **Loopback HTTP unchanged:** Given a loopback `http://` fake or TCP path,
   when Feature 007 offline suites run, then they remain green.
5. **Secrecy on TLS failure:** Given unique markers in headers or arguments,
   when TLS handshake or transport fails, then markers are absent from public
   errors, audit, locks, and logs.
6. **Supply-chain posture:** Given `workbench-mcp/Cargo.toml`, when the default
   suite inspects dependencies, then `reqwest` and `hyper` remain absent and
   `cargo deny` / workspace pins remain compliant.

## Observability

TLS path failures contribute to existing redacted MCP transport error
categories. No new high-cardinality host labels. Operators diagnose via stable
error kinds and server availability pin status.

## Clarifications
