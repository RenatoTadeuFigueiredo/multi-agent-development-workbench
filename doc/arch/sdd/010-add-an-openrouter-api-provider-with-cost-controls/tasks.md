# Tasks: OpenRouter API Provider and Cost Controls

## Task Breakdown

- [x] T001 Extend configuration and schema for API base URL and cost policy:
  `policies.cost.max_session_usd_micros`, optional
  `max_attempt_usd_micros`, provider `base_url`, require cost policy when any
  `type: api` provider is present; refresh generated fixtures.
- [x] T002 Scaffold `workbench-openrouter` with `forbid(unsafe_code)`, modules
  for adapter, transport, budget, credential, protocol, and error; offline unit
  tests compile without network.
- [x] T003 Implement secret resolution port (`SecretSource`) with memory fake
  and platform keyring adapter; fail closed on missing/empty credentials.
- [x] T004 Implement offline fake HTTP Chat Completions streaming transport and
  SSE/delta normalization with 8 MiB bounds and redacted failures.
- [x] T005 Implement session cost ledger and pre-dispatch budget gate with
  attempt estimates and post-success spend recording.
- [x] T006 Implement `OpenRouterProviderAdapter` on the shared provider port
  (capabilities, auth status, session start, prompt stream, cancel, shutdown).
- [x] T007 Compose the adapter in `workbench-daemon` provider runtime for
  `type: api` providers; wire ledger and secret source; keep material-cost
  paid-inference semantics.
- [x] T008 Add Feature 010 acceptance harness in `workbench-testkit` binding
  every Gherkin case to executable evidence; default suite offline only.
- [x] T009 Add `#[ignore]` live smoke test that does not run by default.
- [x] T010 Update STATUS, operations runbook, Makefile acceptance targets, and
  ADR; mark next-ready #17.
- [ ] T011 Run `speckit validate`, `make check` / workspace tests; present for
  human-authorized merge when CI is green (`refs #16`).

## Dependencies

- Features 001–009 on `main`.
- Issue [#16](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/issues/16).
- Spec artifacts: `spec.md`, `plan.md`, ADR 0011, feature CUE, Gherkin.
- No dependency on issue #17 for acceptance of this feature.
