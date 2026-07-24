# Tasks: Orchestration Kernel Foundation

`[P]` marks work that can proceed in parallel after its declared dependencies.
`SC-n` refers to the numbered acceptance scenario in `spec.md`.

## Phase 0 — Change Control and Contract Freeze

- [x] T001 Link GitHub issue `#1`, use the active feature branch from `main`,
  and retain `refs #1` in every implementation commit.
- [x] T002 Freeze feature 001 as a local macOS/Linux vertical slice with fake
  providers, 25 requirements, 23 scenarios, and no live MCP or paid inference.
- [x] T003 Resolve replay cursor, audit actor, one-shot configuration command,
  encrypted export manifest, endpoint, and stale-socket semantics in the
  governing corpus.

## Phase 1 — Rust Workspace and Contract Baseline

- [ ] T004 Create `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, and
  the seven crates declared in `plan.md`, pinning Rust 1.95.0 and every direct
  dependency.
- [ ] T005 [P] Configure workspace formatting, warnings-denied Clippy,
  unsafe-code policy, release profiles, license metadata, and baseline
  `build`, `fmt`, `lint`, and `test` targets in `Makefile`.
- [ ] T006 [P] Generate checked-in fixtures from AsyncAPI, the five JSON
  Schemas, CUE routing objects, and the statechart under
  `crates/workbench-testkit/fixtures/generated/`; add
  `scripts/check-contract-drift.sh` to reject drift. [FR-009, FR-011, FR-018]

## Phase 2 — Core Domain

- [ ] T007 Implement UUIDv7 identifiers, sequences, cursors, hashes, validated
  value objects, and the closed redacted error taxonomy in
  `crates/workbench-core/src/{identity,value,error}.rs`. [FR-005, FR-016]
- [ ] T008 Implement domain events, causation metadata, effect classes,
  attempt facts, sensitive/public payload separation, and provider,
  coordinator, clock, event-store, telemetry, and registry ports in
  `crates/workbench-core/src/{event,ports}.rs`. [FR-001, FR-009, FR-020]
- [ ] T009 Implement legal session states, deterministic folding, serialized
  controls, append-only redirect, and idempotent control outcomes in
  `crates/workbench-core/src/session.rs`. [FR-012, FR-013; SC-8, SC-9]
- [ ] T010 [P] Implement attempt lifecycle, five-second cancellation,
  conservative retry eligibility, `outcome_unknown`, and reconciliation in
  `crates/workbench-core/src/attempt.rs`. [FR-013, FR-020; SC-14, SC-15]
- [ ] T011 [P] Implement the side-effect-free ordered routing chain and
  single-executor routing plan in `crates/workbench-core/src/routing.rs`.
  [FR-006–FR-008; SC-4, SC-5]
- [ ] T012 [P] Implement monotonic policy intersection, protected-action
  classification, peer-derived actors, and idempotent approval decisions in
  `crates/workbench-core/src/policy.rs`. [FR-014, FR-015, FR-024; SC-16, SC-17]
- [ ] T013 Implement orchestration services that persist input, route,
  preflight, plan, and attempt facts before side effects in
  `crates/workbench-core/src/orchestrator.rs`. [FR-001, FR-005–FR-010, FR-020]
- [ ] T014 [P] Add unit and property tests for folding, sequences, controls,
  routing order, retry safety, monotonic policy, and request deduplication in
  `crates/workbench-core/tests/`. [SC-4, SC-5, SC-8, SC-9, SC-14–SC-17, SC-22]

## Phase 3 — Configuration, Locks, and Preflight

- [ ] T015 Implement schema-matching configuration models and exact safe
  defaults for the fake-provider profile in
  `crates/workbench-config/src/model.rs` and `fixtures/builtins.yaml`.
- [ ] T016 Implement safe macOS/Linux source discovery and independently
  validated built-in, user, repository, and session-layer merge semantics in
  `crates/workbench-config/src/{source,merge}.rs`. [FR-002; SC-2, SC-3]
- [ ] T017 [P] Implement cross-reference validation, aliases, central MCP/tool
  grants, capability requirements, and forbidden idempotency classifications
  in `crates/workbench-config/src/validate.rs`. [FR-003, FR-014, FR-015]
- [ ] T018 Implement schema-driven redaction, source attribution, canonical
  JSON, and BLAKE3 snapshot hashing in
  `crates/workbench-config/src/snapshot.rs`. [FR-004; SC-2]
- [ ] T019 Implement deterministic base and linked session lock generation,
  verification, executable SHA-256, and override restrictions in
  `crates/workbench-config/src/lock.rs`. [FR-018; SC-2]
- [ ] T020 Implement alias resolution, capability preflight, and ordered
  compatible fallback selection in `crates/workbench-config/src/preflight.rs`.
  [FR-003, FR-009, FR-010; SC-6, SC-7]
- [ ] T021 [P] Add configuration, redaction, cross-reference, capability, and
  byte-identical lock tests under `crates/workbench-config/tests/`.
  [SC-2, SC-3, SC-6, SC-7, SC-16]

## Phase 4 — Encrypted Storage and Recovery

- [ ] T022 Create the forward-only initial SQLite migration and safe open,
  WAL, integrity, schema-version, and transaction boundary in
  `crates/workbench-storage/{migrations,src/sqlite}/`. [FR-005, FR-019, FR-025]
- [ ] T023 [P] Implement XChaCha20-Poly1305 with CSPRNG nonces, authenticated
  schema/session/event metadata, and zeroized buffers in
  `crates/workbench-storage/src/crypto.rs`. [FR-019; SC-18]
- [ ] T024 Define `KeyStore` and `KeyManager`, fixed record attributes,
  per-session envelopes, root rotation, zeroizing cache, orphan enumeration,
  and deterministic memory adapter in
  `crates/workbench-storage/src/key_store/`. [FR-017, FR-019]
- [ ] T025 [P] Implement non-synchronizable macOS Keychain records and
  fail-closed error mapping in `key_store/macos.rs`. [FR-019, FR-022; SC-19]
- [ ] T026 [P] Implement Linux Secret Service login-collection records and
  fail-closed error mapping in `key_store/linux.rs`. [FR-019, FR-022; SC-19]
- [ ] T027 Implement encrypted transactional append, monotonic sequence
  allocation, ordered replay, folded state, and durable command outcomes in
  `crates/workbench-storage/src/{encrypted_event_store,command_outcomes}.rs`.
  [FR-005, FR-011, FR-019, FR-025; SC-1, SC-10, SC-22]
- [ ] T028 Implement atomic session creation across key store and SQLite,
  compensation, base/session lock storage, and startup orphan-envelope cleanup
  in `crates/workbench-storage/src/session_creation.rs`. [FR-004, FR-019]
- [ ] T029 Implement startup recovery of started attempts without definite
  terminal facts and preserve blocked `outcome_unknown` state in
  `crates/workbench-storage/src/recovery.rs`. [FR-013, FR-020; SC-15]
- [ ] T030 Implement terminal-only retention, the resumable deletion journal,
  envelope destruction, cache eviction, purge, and durable tombstone in
  `crates/workbench-storage/src/{retention,deletion}.rs`. [FR-023; SC-20, SC-21]
- [ ] T031 Implement streaming age v1 export with canonical version-1 NDJSON
  manifest, absolute owner-only output, no overwrite/symlink/plaintext
  temporary file, and no key material in
  `crates/workbench-storage/src/export.rs`. [FR-023; SC-21]
- [ ] T032 Add cryptographic vectors, tampered-AAD, nonce, encrypted WAL,
  atomicity, crash-point, retention, deletion, export, and plaintext-canary
  tests under `crates/workbench-storage/tests/`. [SC-18–SC-22]
- [ ] T033 [P] Add a common key-store conformance suite plus target-specific
  macOS Keychain and Linux Secret Service smoke tests; keep platform stores out
  of the default offline suite. [FR-017, FR-019, FR-022; SC-19, SC-23]

## Phase 5 — Protocol, Daemon, and Local IPC

- [ ] T034 Implement method-specific result DTOs and generated wire mappings
  for every AsyncAPI method in
  `crates/workbench-protocol/src/{command,response,event}.rs`; reject unknown
  fields and schema majors. [FR-011, FR-016, FR-025]
- [ ] T035 [P] Implement strict NDJSON parsing, duplicate-key/UTF-8/trailing
  data rejection, domain-error mapping, and the 8 MiB ceiling in
  `crates/workbench-protocol/src/{codec,validation}.rs`. [FR-016, FR-021; SC-12]
- [ ] T036 [P] Implement the 1,024-event/8-MiB bounded subscriber queue,
  at-least-once delivery, stable IDs, and exclusive cursor replay in
  `crates/workbench-protocol/src/subscription.rs`. [FR-011, FR-021; SC-10, SC-13]
- [ ] T037 Implement safe runtime paths, owner-only permissions, symlink
  rejection, conservative stale-endpoint proof, and single-daemon locking in
  `crates/workbench-daemon/src/runtime_paths.rs`. [FR-001, FR-022]
- [ ] T038 Implement same-user Unix sockets, peer credential verification
  before negotiation, version selection, and bounded connection tasks in
  `crates/workbench-daemon/src/ipc/unix.rs`. [FR-011, FR-022; SC-11–SC-13]
- [ ] T039 Compose configuration, lock verification, key store, migrations,
  recovery, registry, orphan cleanup, and graceful shutdown in
  `crates/workbench-daemon/src/{startup,runtime}.rs`.
- [ ] T040 Implement `status.get`, atomic `session.create`, `session.get`,
  `session.attach`, per-session command serialization, and durable
  `request_id` handling in `crates/workbench-daemon/src/application.rs`.
  [FR-004, FR-011, FR-025; SC-10, SC-22]
- [ ] T041 Implement prompt ordering, routing, capability preflight,
  clarification, fake-provider streaming, and attempt persistence in
  `crates/workbench-daemon/src/services/prompt.rs`. [SC-1, SC-4–SC-7]
- [ ] T042 [P] Implement pause, resume, redirect, cancellation deadline, and
  multi-client control fan-out in
  `crates/workbench-daemon/src/services/controls.rs`.
  [FR-012, FR-013; SC-8, SC-9, SC-14, SC-15]
- [ ] T043 [P] Implement approval and reconciliation services that persist the
  peer-derived actor and decision before protected work in
  `crates/workbench-daemon/src/services/reconciliation.rs`. [FR-020, FR-024]
- [ ] T044 [P] Add structured redacted tracing, bounded-label metrics,
  disabled-by-default OTLP, recovery diagnostics, and graceful shutdown tests
  in `crates/workbench-daemon/src/telemetry.rs` and `tests/`.

## Phase 6 — Headless CLI and Deterministic Testkit

- [ ] T045 Implement fake provider, coordinator, clock, key store, tool,
  telemetry sink, fixtures, and fail-fast network denial in
  `crates/workbench-testkit/src/`. [FR-017; SC-23]
- [ ] T046 [P] Add reusable provider and client contract suites in
  `crates/workbench-testkit/src/contracts/` covering discovery,
  authentication, resume, stream, cancel, tool events, normalized failures,
  protocol methods, and result schemas. [FR-009, FR-011, FR-016]
- [ ] T047 [P] Implement the documented CLI tree, protocol client, request
  IDs, cursor handling, human/JSON output, exit codes, and signal ownership in
  `crates/workbench-cli/src/{args,client,output,signals}.rs`.
- [ ] T048 Wire daemon startup, one-shot daemon-backed configuration commands,
  sessions, prompts, controls, approvals, reconciliation, export, deletion,
  and status in `crates/workbench-cli/src/main.rs`.
- [ ] T049 [P] Add CLI command, JSON schema, stderr, exit-code, stdin, UUIDv7,
  and SIGINT contract tests under `crates/workbench-cli/tests/`.

## Phase 7 — Acceptance, Platforms, and Release Evidence

- [ ] T050 Bind SC-1–SC-7 for encrypted execution, configuration, routing, and
  capability behavior in `crates/workbench-testkit/tests/feature_001.rs`.
- [ ] T051 Bind SC-8–SC-10 for shared controls, append-only redirect, and
  cursor replay/deduplication using two independent clients.
- [ ] T052 Bind SC-11–SC-13 for negotiation, unauthorized peers, frame limits,
  and slow-client isolation on macOS and Linux.
- [ ] T053 Bind SC-14–SC-17 for cancellation, uncertain outcomes, monotonic
  policy, and approval ordering with the fake clock and tool.
- [ ] T054 Bind SC-18–SC-21 for encrypted persistence, key-store failure,
  retention, age export, deletion crash recovery, and cryptographic erasure.
- [ ] T055 Bind SC-22–SC-23 for durable request replay and the zero-network,
  zero-paid-quota default suite; fail when any feature scenario is unbound.
- [ ] T056 Add SLO harnesses for 1,000 routes, healthy fan-out, 10,000-event
  replay, controls, and cancellation; update GitHub/GitLab CI for contract
  drift, security/license/SBOM checks, and explicit Linux/macOS gates.
- [ ] T057 Write the executable feature quickstart and update `README.md`,
  `README.pt-BR.md`, `CONTRIBUTING.md`, `AGENTS.md`, operations, and deployment
  docs with real commands, supported platforms, recovery, and test profiles.
- [ ] T058 Run formatting, Clippy, workspace tests, contract suites, platform
  tests, 23/23 Gherkin scenarios, SLOs, plaintext/secret scans,
  `speckit analyze`, `speckit verify`, and `speckit validate`; record sanitized
  evidence before requesting human approval for a pull request.

## Dependencies and Parallel Batches

- T001–T003 precede implementation; T004 precedes every source-code task.
- T007–T013 define the inward domain boundary before infrastructure composes it.
- T015–T020 and T022–T026 may proceed in parallel after T004 and their stated
  core types; T027–T031 require their corresponding configuration and key ports.
- T034–T038 may proceed alongside storage; T039–T044 require configuration,
  storage, protocol, and testkit ports to compile.
- T045 may begin after core ports exist; T046–T049 require their matching
  protocol/daemon surfaces.
- T050–T055 require the vertical slice; T056–T058 are final release gates.
- Platform key-store jobs may be environment-blocked without invalidating the
  default memory-store suite, but release support for that platform remains
  unclaimed until its explicit job passes.
