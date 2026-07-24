# Implementation Evidence

## Baseline

- Date: 2026-07-24
- Branch: `004-add-a-supervised-acp-adapter-for-an-externally-installed`
- Related issue: #7
- Base commit: `4d59f16f03668cbafc6db812431332b98aa5a009`
- Host used for the recorded targeted checks: Darwin 25.5.0 arm64
- Toolchain: Rust and Cargo 1.95.0
- Execution policy: default tests are offline, use explicit fake executables,
  and must not discover installed providers or consume quota

## Delivered Scope

- T001–T009 are implemented locally: bounded ACP framing and full-duplex
  transport, deterministic executable locking, supervised child lifecycle,
  ACP v1 initialization and normalized streaming, conservative cancellation,
  provider-registry composition, the explicit fake ACP process, all 23
  concrete Feature 004 acceptance cases, and the release documentation.
- Pull request #8 recorded all four required GitHub Actions jobs green in run
  30106637866, including macOS, Linux with Secret Service, supply chain, and
  VS Code validation.

## Recorded Targeted Evidence

| Check | Result | Scope |
|---|---|---|
| `cargo test -p workbench-acp --locked` | Passed | 9 codec, protocol, and transport unit tests; successful provider-adapter and supervisor contract binaries; and the live-only test ignored by default |
| `cargo test -p workbench-testkit --test feature_004 --locked` | Passed: 21; ignored: 1 | The offline target validates and executes the repository-owned Feature 004 evidence; the ignored test is live-only |
| `cargo check -p workbench-daemon --all-targets --locked` | Passed | Daemon provider composition compiled for all targets on the recorded macOS host |
| `make check` | Passed | Complete deterministic local offline gate, including acceptance, SLO, contracts, lint, and Speckit gates |
| `make supply-chain-ci` | Passed | Local advisory, license/source, secret, workflow-policy, and reproducible eight-crate SBOM gates |
| [PR #8, GitHub Actions run 30106637866](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/actions/runs/30106637866) | Passed: 4/4 | macOS check: 3m27s; Linux with Secret Service: 5m18s; supply chain: 3m29s; VS Code: 14s |
| Final P0/P1 review | Passed | No open P0 or P1 findings |
| `speckit validate` | Passed: 0 findings | Feature 004 architecture corpus |
| `scripts/check-contract-drift.sh` | Passed | Generated architecture contract fixtures |
| Local Markdown link check | Passed: 51 files, 0 broken local paths | Repository Markdown paths; external URLs were not checked |
| `git diff --check` | Passed | Whitespace and conflict-marker check after the final documentation update |

The local gate and the required macOS, Linux, supply-chain, and VS Code
pull-request jobs are complete.

## Acceptance Method

The committed Gherkin contains 15 scenario headings, including outlines that
expand into 23 concrete cases with 67 source steps and 103 expanded steps. The
repository-owned Rust runner validates a stable fingerprint and step count for
every concrete case, then executes the 11 distinct evidence tests referenced by
those cases using exact test selection. This evidence crosses the application,
provider adapter/runtime, supervisor, transport, and fake-process layers.

Speckit 0.18.10 verification remains advisory for Features 001 and 004. It
reports zero loaded bindings because its executable registry does not load the
repository's external Rust tests. The repository-owned runner is the
authoritative acceptance gate; the advisory count is not a test failure.

The final boundary hardening also proves that newline discovery resumes from
the previous buffer extent, so an exact 8 MiB frame is accepted without
quadratic rescanning and a one-byte-oversized frame is rejected. Cancellation
uses a 4.5-second provider budget plus a 500-millisecond daemon finalization
reserve within the public five-second deadline.

## Release Evidence

- [x] Fingerprint all 23 concrete Feature 004 cases and execute their 11
  repository-owned evidence tests across the application, adapter, supervisor,
  transport, and fake-process layers.
- [x] Record green macOS and Linux CI results in PR #8, GitHub Actions run
  30106637866.
- [x] Run and record the final local `make check` after all implementation and
  documentation changes are stable.
- [x] Run the local release supply-chain checks and record advisory,
  license/source, secret-scan, workflow-policy, and SBOM results.
- [x] Record final P0/P1 code-review disposition.
- [x] Record the optional live handshake-only smoke as passed or explicitly
  skipped by the operator.

## Live Smoke

Status: **passed on the recorded macOS review host**.

The strong production-path smoke ran the bounded `--version` probe, verified
the lock, retained the digest-pinned private snapshot, launched
`grok agent --no-leader stdio` with `GROK_DISABLE_AUTOUPDATER=1`, completed one
compatible ACP v1 `initialize`, and reaped the child. Grok Build reported
`0.2.111 (94172f2aa4e5)`; its optional `agentInfo` was absent. The test sent
neither `session/new` nor `session/prompt` and consumed no inference quota.

The observed executable and signing details are recorded in the
[supply-chain review](../../../../docs/security/grok-acp-supply-chain-review.md).
Its upstream distribution provenance remains unverified, so this is
compatibility evidence rather than artifact approval.
