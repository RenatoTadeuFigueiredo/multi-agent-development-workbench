# Tasks: Supervised Codex Subscription Adapter

## Task Breakdown

- [x] T001 Scaffold the `workbench-codex` crate in the workspace: add the
  member to the root `Cargo.toml`, create `crates/workbench-codex` with
  `forbid(unsafe_code)`, public constants `MAX_FRAME_BYTES` (8 MiB) and
  `CODEX_EXEC_JSONL_PROTOCOL` (`codex-exec-jsonl/1`), modules
  `{adapter,codec,error,process,protocol}`, and unit-test targets that compile
  offline without network or installed Codex.
- [x] T002 Extend configuration schema and Rust models with explicit
  `driver: codex` for `subscription-cli`: update
  `ProviderDriver`, YAML/JSON schema, builtins/examples, and semantic
  validation so `codex` requires an absolute executable, remains independent of
  `claude-code`, and keeps backward-compatible safe defaults for existing
  Claude providers.
- [x] T003 Wire lock generation, private snapshot, and startup identity for the
  Codex driver: pin protocol `codex-exec-jsonl/1`, normalized CLI version, and
  executable SHA-256; run bounded `--version` and `login status` probes only
  against the private snapshot; fail closed on digest mismatch, symlink, unsafe
  path components, or probe failure before provider dispatch.
- [x] T004 Implement the strict UTF-8 JSONL codec with an 8 MiB encoded frame
  ceiling excluding newline: reject duplicate keys, empty frames, invalid
  UTF-8, malformed JSON, incomplete frames, invalid event shapes, and oversized
  frames with stable redacted errors; accept additive unknown fields only after
  required fields validate.
- [x] T005 Implement process supervision and the fixed launch profile: direct
  argv `codex exec --json --ephemeral --sandbox read-only -C <workspace>
  -m <model> <prompt>` with canonical workspace as cwd and `-C`; strip
  `OPENAI_API_KEY`, `CODEX_API_KEY`, alternate base-URL, and OSS/local provider
  selectors; never pass approval-bypass, writable sandbox, full-auto, `--oss`,
  or session resume flags; drain stderr on a bounded discard path.
- [x] T006 Implement ChatGPT-subscription preflight and authentication policy:
  accept only bounded `login status` evidence of ChatGPT login; treat missing
  login, API-key login, and unknown modes as unavailable before prompt start;
  never open, copy, or log `CODEX_HOME` credential files (including
  `auth.json`); never run `codex login`, `logout`, `update`, or installers.
- [x] T007 Implement the provider-port adapter: one fresh child per Workbench
  attempt and workspace; normalize the pinned event subset
  (`thread.started`, `turn.started`, `turn.completed`, `turn.failed`,
  `item.*` required for text and bounded tool naming, `error`); emit ordered
  content without duplicate visible text; map definite completion and fail-
  closed structured failure; treat EOF/crash/malformed loss after
  `dispatch_started` as `outcome_unknown` with no automatic retry; keep
  thinking, usage bodies, raw tool I/O, provider session ids, and frames off
  durable surfaces.
- [x] T008 Implement cancellation and shutdown for one-shot `exec`: confirm
  cancellation only on a documented abort/cancelled terminal event for the
  active attempt before reaping; otherwise terminate and reap the process
  group and return unconfirmed within the 4.5 s provider budget (500 ms
  daemon finalization reserve); shutdown rejects new work, escalates kill,
  drains pipes, and proves reaping—unreaped children are startup/shutdown
  failures, not success.
- [x] T009 Compose Codex with ACP and Claude through the same daemon registry,
  catalog, startup, live health, configuration-lock, and shutdown paths
  without provider conditionals in the domain; expand the managed-lifecycle
  enum and provider wiring so three adapters coexist.
- [x] T010 Add a committed `fake_codex` executable covering version, ChatGPT
  vs API-key vs missing vs unknown auth, successful streaming, malformed and
  oversized frames, sandbox/tool containment observations, confirmed abort
  terminal event, hang/unconfirmed cancel, crash after dispatch, preflight
  failure, credential-path non-access, and shutdown profiles without PATH
  discovery or network.
- [x] T011 Add unit and crate-level tests in `workbench-codex` for frame
  boundaries (exact 8 MiB accept / +1 reject), event parsing, deduplicated
  text, auth classification, launch argv and environment sanitization,
  cancellation confirmation rules, and redacted error categories.
- [x] T012 Implement the Feature 006 acceptance runner in
  `workbench-testkit`: fingerprint all concrete Gherkin cases from
  `add-a-supervised-codex-subscription-adapter-that-pins-an.feature`
  (including Scenario Outline expansions; target 23 bindings), bind each to
  real application/adapter/supervisor/fake evidence, and prove offline
  zero-quota, zero-network, no installed-Codex discovery, and no credential
  store access.
- [x] T013 Prove secret containment across success and failure paths: unique
  markers in auth output, environment, stderr, tool data, thinking, usage,
  and provider identifiers must be absent from replies, telemetry, logs,
  locks, SQLite, WAL, encrypted export, and public health payloads.
- [x] T014 Update threat model, operations, deployment/runbook, README and
  project guides, configuration examples, and an English/PT-BR Codex provider
  runbook disclosing OpenAI-controlled subscription eligibility, billing, and
  that Workbench never brokers credentials.
- [x] T015 Add an ignored prompt-free live compatibility smoke for an explicit
  executable path: bounded `login status` (ChatGPT) and `--version` identity
  only; no model turn, no inference, no default-suite inclusion; document
  operator authorization requirements.
- [x] T016 Run formatting, Clippy (`-D warnings`), workspace tests, Features
  001–006 acceptance, SLO, Speckit validate/analyze/verify, platform (when
  applicable), supply-chain, secret, policy, and documentation-link gates;
  record immutable implementation evidence under this feature directory.

## Dependencies

- Features 001, 004, and 005 supply the provider port, durable attempt
  semantics, executable locking, private snapshots, cancellation reserve,
  daemon registry patterns, and fake-process harness conventions.
- Codex CLI `0.145.x` is the research floor for `exec --json`, `--ephemeral`,
  `--sandbox read-only`, `-C`, and ChatGPT `login status`; the repository lock
  pins the exact tested executable, not merely the minor line.
- Default development and CI do not require Codex CLI, authentication, network
  access, or provider quota.
- Live compatibility requires an explicitly configured, already authenticated
  official Codex installation and remains opt-in and prompt-free.
- Final review includes: ChatGPT-only auth fail-closed, API-key environment
  stripping, credential-file non-access, read-only launch profile, frame
  boundary, ordered cancellation confirmation, process-group reaping, and
  coexistence with ACP and Claude.

## Parallelism

- T001 may start immediately.
- T002 and T003 may proceed in parallel after T001 exists for workspace
  membership; lock wiring (T003) depends on the driver enum from T002.
- T004 and T005 may proceed in parallel after T001.
- T006 depends on T003 and T005.
- T007 depends on T004–T006.
- T008 depends on T005 and T007.
- T009 depends on T002, T003, T007, and T008.
- T010 may proceed in parallel with T004–T008 once launch/auth contracts are
  frozen in the plan and Gherkin feature.
- T011 depends on T004–T008.
- T012 and T013 depend on T009–T011 and T010.
- T014 may proceed in parallel with implementation once behavior is stable;
  finish before release evidence.
- T015 depends on T007 and documentation from T014.
- T016 completes last after T012–T015.
