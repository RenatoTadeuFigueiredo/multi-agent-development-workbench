# Implementation Plan: Add a Versioned Session List Command to the Local Workbench

## Overview

Add a read-only, paginated `session.list` command to the existing
`workbench/1` local IPC protocol. The daemon reads an indexed page of persistent
session metadata; the CLI exposes it non-interactively; and the VS Code bridge
uses it only for a workspace-local session picker.

## Technical Approach

The protocol crate defines `SessionList` parameters and a metadata-only result.
Storage provides deterministic keyset pagination using an exclusive session-ID
cursor without decrypting event content. The daemon treats listing as a
daemon-scoped read command and returns the protocol DTO. The CLI maps a new
`session list` subcommand to that DTO.

The VS Code protocol transport adds short-lived request helpers for
initialization, create, and list. The extension resolves one endpoint from the
active workspace configuration, creates a persistent session for **New
Session**, and presents summaries from `session.list` in a Quick Pick for
**Select Session**. The existing manual attach-by-ID workflow remains
available. No client owns orchestration, global discovery, storage, or
provider policy.

The AsyncAPI contract, CUE value object, domain semantics, feature spec, ADR,
and generated contract fixture capture the versioned surface. Rust and
TypeScript tests use deterministic fakes and run offline.

## Companion Artifacts

The following optional companion files may be created alongside this
plan to capture additional context:

- `data-model.md` is unnecessary: the feature reuses the persistent session
  record and publishes a projection rather than a new domain entity.
- `contracts/` is represented by the versioned AsyncAPI and CUE artifacts in
  `doc/arch/contracts` and `doc/arch/schemas`.
- `quickstart.md` is represented by the VS Code extension README, which
  documents the workspace-local workflow.
