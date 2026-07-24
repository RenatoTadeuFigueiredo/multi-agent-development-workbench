# Grok ACP Provider Supply-Chain Review Record

## Review Status

This record separates controls implemented by Workbench from evidence that must
be collected for a particular operator-installed Grok Build executable.

| Field | Recorded value |
|---|---|
| Review scope | Feature 004 supervised ACP adapter and update process |
| Repository implementation | External process boundary; no linked Grok private crates |
| Approved external artifact | Not recorded by the repository |
| External version and SHA-256 | Workstation-local lock output; intentionally not committed here |
| Distribution provenance | Must be recorded by the operator during update review |
| Repository supply-chain gate | Local `make supply-chain-ci` passed; PR #8 supply-chain job passed in GitHub Actions run 30106637866 (3m29s) |
| Exact-profile live handshake | Passed on the recorded macOS review host |

The absence of an approved artifact in this record is intentional. Workbench
accepts an explicitly configured executable after local validation; it cannot
prove that an arbitrary file came from an official distribution channel.

## Recorded macOS Compatibility Observation

On 2026-07-24, the user-local canonical Grok Build executable reported
`0.2.111 (94172f2aa4e5)` and SHA-256
`e1fafdfffe14f339460befaf194360e8f90bfd02efe8a4f24cfa1c7aea657ffe`.
macOS `codesign --verify --deep --strict` passed, with Developer ID Application
`X.AI Corporation (5Y6N3AJ54S)`. The redacted ignored test completed ACP v1
`initialize`, observed available authentication and the required load-session
capability, then reaped the child. It did not create a provider session or send
a prompt, so it consumed no inference quota.

The observed Grok response omitted optional `agentInfo`; executable identity
therefore comes from the bounded `--version` probe and digest-pinned private
snapshot. The [public upstream repository](https://github.com/xai-org/grok-build)
is Apache-2.0 licensed, but it exposes no release/tag matching this binary and
the reported build identifier is not a public upstream commit. Distribution
provenance remains unverified and this observation is compatibility evidence,
not artifact approval.

## Implemented Boundary

Feature 004 does not download Grok Build, invoke a package manager, link private
provider crates, embed authentication code, or maintain a provider runtime
fork. The Grok-derived terminal fork is a separate dependency and trust
boundary.

Workbench opens one absolute executable after verifying that it is canonical,
owned by the current user, executable, non-symlink, and not group/world
writable. It hashes and retains a private snapshot for the validated launch,
then invokes it directly with fixed arguments `agent --no-leader stdio`.
`GROK_DISABLE_AUTOUPDATER=1` disables updates in the supervised child.

Before spawn, the base lock binds configuration, `acp/1`, the bounded
`--version` result, and executable SHA-256. After spawn but before availability
or dispatch, ACP initialization checks protocol, authentication state, required
capabilities, and optional `agentInfo.version` when the agent advertises it.

JSON-RPC frames, reverse requests, and diagnostics remain untrusted. Frames are
limited to 8 MiB, malformed envelopes fail closed, and reverse permission
requests are always denied.

## Evidence to Record for an Update

Record the following outside logs that may contain provider or workspace data:

| Evidence | Required record |
|---|---|
| Distribution | Approved source URL or package channel and retrieval date |
| Authenticity | Vendor signature/checksum result when one is published |
| Identity | Previous and candidate version plus SHA-256 |
| Change review | ACP, authentication, updater, permission, tool, plugin, hook, MCP, session, and telemetry changes |
| Repository gates | Commit SHA and results of `make check` and release supply-chain checks |
| Compatibility | Sanitized exact-profile handshake result |
| Rollback | Retained previous executable and matching lock or supported recovery method |

The repository's default gate uses only the explicit fake ACP executable and
Cargo's locked offline dependency graph. `make supply-chain-ci` separately
combines the network-enabled advisory check, offline license/source policy,
secret scan, pinned workflow policy, and reproducible SBOM generation.

## Update Procedure

1. Finish or reconcile active attempts and stop the workspace daemon.
2. Obtain the candidate through the approved channel and collect the evidence
   above.
3. Confirm that it supports `agent --no-leader stdio` without a shell.
4. Run `workbench config lock`, then `workbench config validate`.
5. Run the repository offline and release supply-chain gates.
6. Run the optional handshake-only live smoke separately.
7. Start the daemon and confirm bounded adapter health before the first prompt.

Restore the previous executable and matching lock if any check fails. Never
edit a digest manually, bypass compatibility, or downgrade during an active
attempt.

## Residual Risk

A matching digest proves identity relative to the local lock, not vendor
authenticity or benign behavior. The child runs as the current user, receives
the canonical workspace as its working directory, and may use Grok-owned
configuration or network behavior. Feature 004 is process supervision, not an
operating-system sandbox.

Operators must review native Grok plugins, hooks, skills, MCPs, and
configuration before handling confidential data. `--no-leader` prevents global
leader session sharing but does not disable those extension surfaces. The ACP
client advertises no filesystem or terminal authority and denies reverse
permission requests, yet OS-level containment requires a separate decision.

Authentication material remains in Grok-owned storage. Workbench does not
collect it for provenance, debugging, SBOMs, locks, telemetry, or support
bundles. Raw stdio and provider session identifiers are equally excluded from
evidence.
