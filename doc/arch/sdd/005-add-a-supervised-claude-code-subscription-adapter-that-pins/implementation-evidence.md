# Implementation Evidence

## Baseline

- Date: 2026-07-24
- Branch:
  `005-add-a-supervised-claude-code-subscription-adapter-that-pins`
- Related issue: #9
- Base commit: `2fa27a5f32b0bb70a534f40058dbf8c564d4b7f6`
- Host used for the recorded checks: Darwin 25.5.0 arm64
- Toolchain: Rust and Cargo 1.95.0; Speckit 0.18.10
- Execution policy: default tests are offline, use the committed fake Claude
  executable, and neither discover installed providers nor consume quota

## Delivered Scope

- T001–T011 are implemented locally: explicit `claude-code` driver
  configuration, executable locking, strict 8 MiB stream-JSON framing,
  subscription-auth preflight, a fixed read-only launch profile, one supervised
  process per prompt, conservative cancellation and result classification,
  provider-registry composition, a deterministic fake process, all 27 concrete
  Feature 005 cases, secret-containment evidence, and operator documentation.
- Cancellation during setup is observable inside the public five-second
  deadline. Process groups, bounded output buffering, ordered interrupt
  acknowledgement, durable uncertainty, descendant-free lock probes, and
  workspace-scoped shutdown were hardened during the final architecture
  review.
- Pull-request and GitHub Actions evidence are intentionally not claimed before
  review and publication.

## Recorded Evidence

| Check | Result | Scope |
|---|---|---|
| `cargo test -p workbench-claude --locked` | Passed; live: 1 ignored | 12 codec/protocol/adapter unit tests, the custom provider process harness, and the opt-in live test |
| `cargo test -p workbench-daemon --locked` | Passed: 78 | Provider composition, durable lifecycle, cancellation, descendant reaping, and shutdown |
| `cargo test -p workbench-testkit --test feature_005 --locked` | Passed: 10 | All 27 fingerprinted concrete Feature 005 cases execute repository-owned evidence |
| `make check` | Passed | Formatting, Clippy with warnings denied, contracts, workspace tests, Features 001–005, SLOs, and Speckit |
| `make test-platform` | Passed: 1 | Real macOS Keychain contract with isolated setup and cleanup |
| `make supply-chain-ci` | Passed | Policy, secrets, RustSec advisories, licenses, sources, and nine CycloneDX 1.5 SBOMs |
| `scripts/check-contract-drift.sh` | Passed | Generated architecture contract fixtures |
| Local Markdown link check | Passed: 55 files, 36 local targets, 0 broken | Repository Markdown excluding dependencies; external URLs were not checked |
| Final multi-agent review | Passed | Architecture, acceptance evidence, and operations documentation; no open P0/P1 findings |

The final serialized SLO gate recorded confirmed cancellation at 7.513 ms,
unconfirmed cancellation at 4509.198 ms, routing p95 at 1.138 ms, healthy IPC
p95 at 0.550 ms, control acknowledgement p95 at 0.582 ms, and encrypted replay
of 10,000 events at 860.918 ms.

## Acceptance Method

The committed Gherkin contains 16 scenario headings, including outlines that
expand into 27 concrete cases with 48 source steps and 81 expanded steps. The
repository-owned Rust runner verifies stable fingerprints and explicitly binds
every case to executable application, adapter, supervisor, codec, fake-process,
storage, or configuration evidence.

Speckit 0.18.10 verification remains advisory for the external Rust acceptance
runners. It reports 62 unbound scenarios because its executable registry does
not load those bindings. The repository-owned runners are the authoritative
acceptance gate; `speckit validate` reports zero findings.

## Live Smoke

Status: **explicitly skipped on the recorded host**.

Claude Code 2.1.218 is installed through a convenience symlink, but
`claude auth status` reports `loggedIn: false` and `authMethod: none`. The
ignored smoke requires an explicitly selected real, non-symlink executable and
an already authenticated official installation. No prompt or model turn was
sent, and no provider quota was consumed.

The smoke proves only the bounded authentication, initialization, interrupt
receipt, and process-reaping path. It does not prove a prompt/model turn, lock
regeneration, executable digest, snapshot provenance, or billing eligibility.

## Release Evidence

- [x] Execute all 27 concrete Feature 005 cases through repository-owned
  evidence without provider discovery, network access, or quota.
- [x] Run the complete deterministic local gate and real macOS key-store
  contract.
- [x] Run advisory, license/source, secret, policy, and reproducible SBOM
  checks.
- [x] Validate local Markdown links and generated architecture contracts.
- [x] Resolve final architecture, acceptance, security, and operations review
  findings.
- [x] Record the optional live smoke as explicitly skipped with the reason and
  unproven boundaries.
