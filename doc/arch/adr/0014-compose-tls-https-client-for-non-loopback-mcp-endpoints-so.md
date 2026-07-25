---
status: accepted
date: 2026-07-25
deciders: [maintainer]
consulted: []
informed: []
---

# Compose a Minimal rustls TLS Client for Non-Loopback MCP HTTPS

## Context and Problem Statement

Feature 007 requires TLS for non-loopback HTTP MCP endpoints and rejects
cleartext non-loopback URLs. The `workbench-mcp` HTTP client implemented a
manual HTTP/1.1 loopback path and deliberately failed closed for non-loopback
HTTPS because no TLS stack was composed. Operators cannot reach remote pinned
MCP servers until that gap closes (GitHub issue #30).

Supply-chain policy forbids casual HTTP client bloat: Feature 007 acceptance
asserts that `workbench-mcp` does not depend on `reqwest` or `hyper`. Feature
010 similarly prefers a tiny custom client with carefully pinned rustls if a
live HTTPS path is required.

## Decision Drivers

- Close the Feature 007 residual gap for non-loopback HTTPS MCP.
- Keep offline default tests (fake or local TLS fixture only).
- Prefer pure-Rust TLS with workspace-pinned crates and `deny.toml` licenses.
- Avoid introducing `reqwest` / `hyper` into `workbench-mcp`.
- Preserve pin identity, redirect rejection, size ceilings, and redaction.

## Considered Options

- Compose `rustls` + `tokio-rustls` (+ web roots) around the existing manual
  HTTP/1.1 request/response framing.
- Add `reqwest` or `hyper` with rustls backend for a full HTTP client.
- Keep failing closed and document remote MCP as unsupported.
- Use `native-tls` / system OpenSSL bindings instead of pure Rust.

## Decision Outcome

Chosen option: **compose `rustls` + `tokio-rustls` with a web root store around
the existing manual HTTP framing**, because it matches the Feature 007/010
supply-chain posture, reuses the pinned-identity client already shipped, and
unblocks non-loopback HTTPS without a large HTTP stack.

Implementation shape:

1. Route every `https://` MCP invoke through a TLS-wrapped TCP stream.
2. Keep cleartext TCP for loopback `http://` only.
3. Production verification uses web PKI roots; tests inject a custom
   `RootCertStore` / client config for a local self-signed fixture.
4. Live public HTTPS remains optional and `#[ignore]` by default.
5. Daemon production composition enables the non-fake HTTP client so pins can
   dial HTTPS when configured.

### Consequences

- Good: non-loopback MCP HTTPS works under the same gateway policy model.
- Good: dependency surface stays small and pure-Rust-first.
- Good: offline TLS fixture proves the path without public network.
- Bad: manual HTTP/1.1 framing remains limited (no HTTP/2, limited redirect
  following); acceptable for the narrow MCP invoke contract.
- Bad: operators with private CAs need a future config surface; tests can
  inject roots but production starts with web PKI only.
