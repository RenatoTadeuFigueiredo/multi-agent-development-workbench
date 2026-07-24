# Implementation Evidence

## Baseline

- Date: 2026-07-23
- Branch: `001-build-the-workbench-orchestration-kernel-foundation-as-a`
- Base commit: `f16f5d0`
- Host: Darwin 25.5.0 arm64
- Toolchain: Rust and Cargo 1.95.0
- Execution policy: default tests offline with fake adapters and no paid
  provider calls

## Release Gates

`make check` completed successfully after the final integrated changes. It
verified rustfmt, workspace Clippy with warnings denied, contract drift,
workspace tests, the client contract, acceptance, SLOs, supply-chain policy,
secret scanning, and all Speckit gates. The workspace run passed 163 tests;
the four explicit SLO tests and the platform credential-store test are ignored
only in the default profile and were executed separately.

The acceptance harness parsed the committed Gherkin corpus, proved exactly 23
unique scenario bindings and fingerprints, and executed all 23 contracts.
The live client contract exercised all 14 protocol methods over Unix IPC,
including event replay and request correlation.

## Performance Evidence

| Objective | Result | Limit |
|---|---:|---:|
| Routing plan, 1,000 durable routes | p95 0.865 ms | p95 100 ms |
| Healthy event fan-out, 1,000 samples | p95 0.459 ms | p95 100 ms |
| Control acknowledgement, 1,000 samples | p95 0.475 ms | p95 100 ms |
| Encrypted replay, 10,000 events | 806.236 ms | 2,000 ms |
| Unconfirmed cancellation | 4,506.383 ms | 5,000 ms |

Fan-out is measured conservatively from the event timestamp immediately before
the durable transaction through receipt by the attached IPC client.

## Security and Recovery Evidence

The real macOS Keychain contract passed with a production-shaped, isolated key
namespace and cleaned up its test credentials. Fault-injection and reopen tests
cover session creation, export publication, deletion, cancellation, retention,
credential-catalog rollback, database clones, and daemon shutdown. Three final
read-only review passes reported no remaining P0 or P1 findings.

RustSec passed with no waiver after pinning fixed `anyhow`, `bytes`, `rand`, and
`time` releases. License and source policy passed offline. The secret scan found
no high-confidence plaintext secret, and seven deterministic CycloneDX 1.5
SBOM documents were generated and structurally validated.

## Speckit Evidence

- `speckit status`: implemented, complete, coherent, locked, no blockers.
- `speckit analyze`: one consistent feature and zero ADR overlaps.
- `speckit validate`: zero findings.
- `speckit verify`: exited successfully and reported the 23 project scenarios
  as advisory `unbound` because its executable registry does not load external
  Rust step bindings. The repository-owned executable harness described above
  is the authoritative 23/23 implementation check.

One non-blocking constraint remains explicit: `KeyManager` is internal and
serialized through `LockedStorage`; direct concurrent rotation must not be
exposed without adding an explicit synchronization contract.
