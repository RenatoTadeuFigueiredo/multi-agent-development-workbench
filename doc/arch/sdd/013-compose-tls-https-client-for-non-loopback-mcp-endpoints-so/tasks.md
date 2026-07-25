# Tasks: MCP Non-Loopback HTTPS TLS Client

- [x] T001 Complete Speckit corpus (spec/plan/tasks/feature/CUE/ADR) and
  advance clarify → plan → analyze as required
- [x] T002 Pin workspace TLS crates (`rustls`, `tokio-rustls`,
  `rustls-native-certs`, `rustls-pki-types`; `rcgen` dev-dep) without
  `reqwest`/`hyper`
- [x] T003 Implement HTTPS TLS path in `workbench-mcp` HTTP client; remove
  non-loopback fail-closed stub; keep loopback HTTP cleartext
- [x] T004 Add offline TLS fixture tests + secrecy/redirect/cleartext cases;
  optional `#[ignore]` live HTTPS smoke
- [x] T005 Enable production daemon HTTP client (`offline_http: false`) with
  TLS composed; keep test harnesses offline
- [x] T006 Feature 013 acceptance harness + STATUS Known Gaps update
- [ ] T007 `speckit validate`, `make check`, CI green, PR refs #30, merge
