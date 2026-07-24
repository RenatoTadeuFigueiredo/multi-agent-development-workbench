# Tasks: Supervised Claude Code Subscription Adapter

## Task Breakdown

- [x] T001 Update the configuration schema and Rust model with an explicit
  `claude-code` driver for `subscription-cli`, executable requirements, semantic
  validation, examples, and backward-compatible safe defaults.
- [x] T002 Generalize executable probing, private snapshots, adapter lock
  identity, and runtime descriptors so ACP and Claude protocols retain their
  own fixed probes and launch profiles.
- [x] T003 Add the `workbench-claude` crate with an 8 MiB strict UTF-8 NDJSON
  codec, duplicate-key rejection, bounded buffering, control correlation, and
  typed parsing for the supported stream-json subset.
- [x] T004 Implement prompt-free initialization, subscription auth preflight,
  fixed child launch, environment sanitization, independent pipe servicing,
  normalized streaming, and bounded error categories.
- [x] T005 Implement one-attempt child ownership, read-only native tool
  containment, definite result classification, interrupt confirmation,
  unconfirmed cancellation, and bounded shutdown/reaping.
- [x] T006 Compose Claude and ACP adapters through the same provider registry,
  catalog, startup, live health, configuration-lock, and daemon shutdown paths
  without provider conditionals in the domain.
- [x] T007 Add a committed fake Claude executable covering version, auth,
  initialization, content, tool, malformed-frame, crash, interrupt, hang, and
  shutdown profiles without PATH discovery or network access.
- [x] T008 Implement the Feature 005 acceptance runner, fingerprint all 27
  concrete Gherkin cases, and bind them to real application, adapter,
  supervisor, transport, and fake-process evidence.
- [x] T009 Prove secret containment across replies, telemetry, logs, locks,
  SQLite, WAL, encrypted export, auth output, environment, stderr, thinking,
  usage, tool data, and provider identifiers.
- [x] T010 Update the threat model, architecture, operations, deployment,
  README, project guides, configuration examples, and an English/PT-BR Claude
  provider runbook, including billing and legal caveats.
- [x] T011 Add an ignored prompt-free live compatibility smoke for an explicit
  executable and document that inference requires separate operator
  authorization.
- [x] T012 Run formatting, Clippy, workspace tests, Feature 001–005 acceptance,
  SLO, Speckit, platform, supply-chain, secret, policy, and documentation-link
  gates; record immutable implementation evidence.

## Dependencies

- Features 001 and 004 supply the provider port, durable attempt semantics,
  executable locking, private snapshots, cancellation reserve, and fake-process
  patterns.
- Claude Code 2.1.214 or newer is the initial compatibility floor because it
  includes reliable drained stream completion; the repository lock pins the
  exact tested executable.
- Default development and CI do not require Claude Code, authentication,
  network access, or provider quota.
- Live compatibility requires an explicitly configured, already authenticated
  official Claude Code installation and remains opt-in and prompt-free.
- Final review includes cancellation during initialization, bounded output
  buffering, ordered interrupt confirmation, and process-group reaping.
