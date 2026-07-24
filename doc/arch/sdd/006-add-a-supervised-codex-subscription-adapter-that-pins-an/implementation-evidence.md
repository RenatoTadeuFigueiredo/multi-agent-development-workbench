# Implementation Evidence: Feature 006

Related issue: #12  
Branch: `006-add-a-supervised-codex-subscription-adapter-that-pins-an`

## Delivered surface

| Area | Path / artifact |
|------|-----------------|
| Adapter crate | `crates/workbench-codex` |
| Protocol pin | `codex-exec-jsonl/1` |
| Frame ceiling | 8 MiB encoded JSONL (excluding newline) |
| Config driver | `ProviderDriver::Codex` (`driver: codex`) |
| Lock identity | version + SHA-256 + protocol via `AdapterInput::codex` |
| Daemon wiring | `ManagedAdapter::Codex`, `AdapterProbeKind::Codex` |
| Fake process | `crates/workbench-testkit/src/bin/fake_codex.rs` |
| Acceptance | `crates/workbench-testkit/tests/feature_006.rs` (23 Gherkin cases) |
| Live smoke | `crates/workbench-codex/tests/live_codex.rs` (`#[ignore]`, prompt-free) |
| Runbooks | `docs/operations/codex-provider.md`, `.pt-BR.md` |
| ADR / CUE / Gherkin | ADR 0007, feature CUE schema, feature Gherkin |

## Task map

T001–T016 marked complete in `tasks.md`.

## Verification notes

Default offline gates must remain network-free and quota-free:

```bash
cargo test -p workbench-codex --locked
cargo test -p workbench-testkit --test feature_006 --locked
make test-codex
make check
```

Live opt-in only:

```bash
WORKBENCH_CODEX_EXECUTABLE=/absolute/path/to/codex \
WORKBENCH_CODEX_VERSION=<pinned> \
cargo test -p workbench-codex --test live_codex -- --ignored --nocapture
```

## Residual gaps (operator / merge)

1. Run Speckit `analyze` / `validate` / `verify` when the binary is available in
   the shell policy environment; corpus and code must stay green.
2. Fill exact FNV-1a scenario fingerprints in `feature_006.rs` (currently
   binding by case name with non-zero runtime fingerprints; set frozen
   constants after one local green run).
3. Update `Cargo.lock` via `cargo generate-lockfile` / `cargo test` after the
   workspace member add.
4. Human approval before opening the PR (`refs #12`, not `closes`).
