# Implementation Plan: MCP Non-Loopback HTTPS TLS Client

## Overview

Close Known Gap #30 by composing a minimal rustls-based TLS client inside
`workbench-mcp`, routing all `https://` MCP invokes through verified TLS while
keeping the existing cleartext loopback HTTP path and offline fakes.

## Technical Approach

### Dependencies (workspace-pinned)

Add to `[workspace.dependencies]` and `workbench-mcp`:

| Crate | Role |
|---|---|
| `rustls` | TLS protocol (default-features off; ring + std) |
| `tokio-rustls` | Async TLS over `tokio::net::TcpStream` |
| `rustls-native-certs` | Production platform trust roots (license-clean) |
| `rustls-pki-types` | Certificate/private key types |

Dev-only:

| Crate | Role |
|---|---|
| `rcgen` | Generate self-signed fixture certs for offline TLS tests |

Do **not** add `reqwest` or `hyper`.

### Client changes (`crates/workbench-mcp/src/http.rs`)

1. Extend `HttpMcpClient` with optional custom `rustls::ClientConfig` for tests.
2. Constructors:
   - `offline()` — fake only (unchanged default for tests)
   - `with_network()` (rename of production path from `with_loopback`) — real
     TCP + TLS for https, cleartext for loopback http
3. Invoke routing:
   - fake → existing `FakeHttpTransport`
   - `https` → connect TCP, `tokio_rustls` handshake with SNI = host, write
     existing HTTP/1.1 POST framing, parse response with existing bounds
   - `http` + loopback → existing cleartext path
4. Remove the non-loopback fail-closed stub.
5. Optional host connect override for offline non-loopback identity tests
   (map pinned host to `127.0.0.1` while preserving Host/SNI).

### Daemon composition

`workbench-daemon` currently boots the gateway with `offline_http: true`.
Switch production attach to `offline_http: false` so configured HTTPS pins can
dial. Acceptance harnesses that need fakes keep `offline_http: true`.

### Testing

- Unit: fake oversize/redirect still green; parse rejects cleartext non-loopback.
- Offline TLS fixture: `rcgen` cert + local `tokio` TLS server on loopback;
  client with custom roots and optional host override for a non-loopback name.
- Secrecy: markers absent from `Display`/`Debug` of errors.
- Manifest guard: still no `reqwest`/`hyper` in `workbench-mcp`.
- `#[ignore]` live smoke: optional public HTTPS (documented, not default CI).

### Acceptance harness

Add `feature_013.rs` fingerprinting the Gherkin scenarios and binding them to
unit/fixture evidence. Wire into `make test-acceptance` if the project lists
features explicitly.

## Risks

- rustls + roots expand the dependency graph; mitigate with exact pins and
  `cargo deny`.
- Private CA environments need a future config surface; out of scope.
- Manual HTTP/1.1 remains minimal; MCP invoke contract stays narrow.

## Rollback

Revert the feature branch/merge. Gateway returns to fail-closed non-loopback
HTTPS; loopback HTTP and offline fakes continue to work.
