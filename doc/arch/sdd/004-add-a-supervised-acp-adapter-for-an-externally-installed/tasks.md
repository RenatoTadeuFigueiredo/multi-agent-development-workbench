# Tasks: Supervised Grok Build ACP Provider Adapter

## Task Breakdown

- [x] T001 Add a generic `workbench-acp` crate with bounded JSON-RPC 2.0 NDJSON
  framing, UUID request correlation, independent read/write progress, strict
  baseline validation, safe handling of additive unknown fields, and
  incremental newline scanning that remains linear at the exact 8 MiB frame
  boundary.
- [x] T002 Require ACP provider executables in configuration; canonicalize and
  validate the target; populate adapter lock inputs; pin protocol, version, and
  SHA-256; and reject startup or session overrides that diverge from the base
  lock.
- [x] T003 Implement the workspace-scoped child supervisor with direct argv,
  canonical working directory, `GROK_DISABLE_AUTOUPDATER=1`, bounded stdio,
  graceful termination, forced cleanup, and guaranteed process reaping.
- [x] T004 Implement ACP version 1 initialization, Grok-owned authentication
  status, `session/new`, `session/load`, `session/prompt`, `session/update`,
  normalized stream mapping, and bounded adapter health.
- [x] T005 Implement fail-closed reverse permission denial and cancellation
  semantics in which only prompt `stopReason: cancelled` confirms cancellation;
  reserve 500 milliseconds of the public five-second deadline for durable
  daemon finalization after the 4.5-second provider budget; keep pause and
  `x.ai/*` interjection unavailable.
- [x] T006 Register configured ACP adapters in daemon composition without
  adding Grok-specific branches to orchestration, and preserve durable
  pre-dispatch, active-crash, shutdown, and `outcome_unknown` behavior.
- [x] T007 Add the explicit offline fake ACP child with modes for successful
  streaming, compatible additive updates, incompatible negotiation, malformed
  and oversized frames, preflight and active crashes, reverse permissions,
  confirmed cancellation, and unconfirmed cancellation.
- [x] T008 Expand the 15 Feature 004 Gherkin headings into 23 fingerprinted
  concrete cases and execute their 11 distinct repository-owned evidence tests
  across application, adapter, supervisor, transport, and fake-process layers,
  including secret-marker inspection, workspace isolation, child reaping, no
  PATH discovery, zero network, and zero quota.
- [x] T009 Extend contract fixtures, acceptance targets, supply-chain review,
  operational documentation, and implementation evidence; run the complete
  local offline gate and the optional handshake-only live smoke test
  separately.

## Dependencies

The feature depends on the existing provider port, deterministic lock,
workspace runtime identity, encrypted event store, five-second cancellation
deadline, and attempt recovery semantics. Automated delivery depends only on
the fake ACP executable. An installed and authenticated Grok Build runtime is
optional and is used solely for a manual handshake-only smoke test.

T001, T002, and the fake-agent portion of T007 may proceed in parallel. T003
depends on the transport boundary from T001. T004 and T005 depend on T001 and
T003. T006 depends on T002 through T005. T008 and T009 complete after the
adapter is registered. Execution on both supported CI operating systems is a
pull-request release gate rather than an implementation task; PR #8 recorded
that gate green on macOS and Linux in GitHub Actions run 30106637866.

Speckit 0.18.10 verification is advisory for this feature because its
executable registry does not load the repository's external Rust tests and
therefore reports zero loaded bindings. The repository-owned fingerprint and
exact-test runner is the authoritative acceptance gate. Its macOS and Linux
pull-request CI evidence is recorded in the implementation evidence.
