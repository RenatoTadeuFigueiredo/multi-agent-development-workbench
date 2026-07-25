---
id: 019f9980-0150-7d4e-9f5a-0150write0001
number: 015
slug: provider-native-write-tools-under-central-policy
status: implement
created_at: 2026-07-25T14:00:00.000000Z
---
# Feature Specification: Provider-Native Write Tools Under Central Policy

Feature: 015-provider-native-write-tools-under-central-policy
Created: 2026-07-25
Related issue: #32

## Objective

Enable Claude Code and Codex provider-native write tools only when central
policy, allowlists, and approval mode permit them—fail-closed by default.

## Scope

Includes:

- `policies.provider_native_writes` with default `mode: disabled`;
- per-provider allowlist for write profiles;
- Claude launch/protocol Write+Edit when allowed;
- Codex workspace-write sandbox and `file_change` items when allowed;
- offline unit and acceptance proof of grant and deny paths.

Excludes:

- reverse permission UI round-trips;
- Bash/shell native tools;
- MCP duplication of gateway policy;
- live write smokes in CI.

## Functional Requirements

1. **FR-015-001:** Default configuration MUST keep provider-native writes off.
2. **FR-015-002:** Writes enable only when mode is `approval-required` and the
   provider id is on the allowlist.
3. **FR-015-003:** Denied paths MUST never accept Write/Edit or file_change.
4. **FR-015-004:** Shared tools remain on the central MCP gateway.

## Success Criteria

- Offline Feature 015 acceptance green.
- STATUS Known Gaps removes #32.
