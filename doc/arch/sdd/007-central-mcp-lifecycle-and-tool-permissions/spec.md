---
id: 019f95fd-ffc1-78f0-926d-624aec387400
number: 007
slug: central-mcp-lifecycle-and-tool-permissions
status: analyzed
created_at: 2026-07-24T21:17:59.105131Z
---
# Feature Specification: Central MCP Lifecycle and Tool Permissions

Feature: 007-central-mcp-lifecycle-and-tool-permissions
Created: 2026-07-24
Related issue: #13

## Objective

Give the Workbench daemon a central MCP and tool gateway that owns installation
identity, lifecycle, role and workflow allowlists, approval gates, redacted
audit, and normalized failure outcomes so every compatible provider sees the
same governed tool surface instead of launching or versioning MCP servers
independently.

## Scope

This feature includes:

- canonical MCP manifest and lockfile shapes for shared servers;
- supervised local stdio MCP children and remote HTTP MCP clients;
- version and digest pinning with fail-closed startup and re-lock rules;
- role and workflow tool allowlists applied before every invocation;
- approval requirements for sensitive, mutating, production, credential, and
  material-cost operations using the existing approval protocol;
- redacted durable audit of planned, allowed, denied, approved, cancelled, and
  failed tool attempts;
- cancellation, crash, and shutdown recovery for gateway-owned MCP processes;
- offline deterministic fakes proving isolation, pinning, allowlists, approvals,
  cancellation, and recovery.

This feature excludes:

- the multi-agent workflow executor (issue #14);
- real-time VS Code workflow control room UX (issue #15);
- OpenRouter provider and cost controls (issue #16);
- the Workbench ACP server and terminal client (issue #17);
- provider-native write, shell, patch, or plugin bridges beyond gateway-exposed
  tools;
- automatic package download from the public internet in default tests;
- embedding MCP credentials in version-controlled configuration;
- production package registries as a required live path for CI.

## User Stories

- As a developer, I want Claude, Codex, Grok, and future API agents to share one
  pinned MCP version so tool behavior does not diverge by provider install.
- As a workflow author, I want role and workflow allowlists to decide which
  tools each step may call so grants stay least-privilege and explainable.
- As a security-conscious operator, I want mutating and sensitive tools to stop
  for explicit approval and leave a redacted audit trail.
- As a local operator, I want MCP binary or package changes to fail closed until
  I regenerate the lock with the new digest and version.
- As a tester, I want default automated suites to exercise the gateway with
  offline fakes only, without network, credentials, or paid quota.

## Functional Requirements

1. **FR-007-001:** The daemon MUST own a central MCP registry derived from
   resolved configuration `mcp_servers` and the deterministic lock `mcps` map.
   Providers and presentation clients MUST NOT install, update, or launch
   shared MCP servers outside that registry.
2. **FR-007-002:** Each MCP server entry MUST declare a stable identifier,
   transport (`stdio` or `http`), version string, and content digest
   (`sha256` of the pinned package, image, or executable artifact). The lock
   MUST pin the same identifier, version, and digest. Empty MCP maps remain
   valid.
3. **FR-007-003:** Canonical manifest fields for a registered server MUST be
   sufficient to launch or connect without provider-specific forks:
   transport, version, digest, and transport-specific endpoint data. Stdio
   servers identify a user-owned absolute executable or package entrypoint;
   HTTP servers identify an absolute HTTPS URL or loopback HTTP URL allowed by
   policy. Credential values MUST reference opaque platform or provider-owned
   secret handles and MUST never appear in manifests, locks, events, or logs.
4. **FR-007-004:** Startup and lock generation MUST verify every configured
   MCP against its lock pin before the gateway becomes available for tool
   dispatch. Digest mismatch, missing artifact, unsafe path, symlink target,
   group/world-writable executable, or unsupported transport MUST fail closed
   with a stable redacted error and MUST leave the server unavailable.
5. **FR-007-005:** Accepting a new MCP package, image, executable, or remote
   endpoint identity MUST require an explicit operator lock regeneration. The
   gateway MUST NOT auto-update pinned servers during a running daemon.
6. **FR-007-006:** Local stdio MCP servers MUST be started and supervised by
   the daemon gateway only. Launch MUST use a direct argv without a shell, a
   private working directory under the daemon runtime root, piped stdio, and
   bounded stderr discard. One supervised child identity MUST NOT be shared
   across workspaces.
7. **FR-007-007:** Remote HTTP MCP servers MUST be reached only through the
   gateway using the pinned endpoint identity. The gateway MUST enforce TLS
   for non-loopback hosts, reject redirects to unpinned hosts, and apply
   bounded request and response size limits.
8. **FR-007-008:** Tools of kind `mcp` MUST reference a registered
   `mcp_server` and one or more named operations. Each operation MUST declare
   `effect_class`, `idempotent`, `material_cost`, and `approval` using the
   existing configuration schema. Semantic validation MUST reject missing
   server references and illegal `idempotent: true` combinations for paid
   inference, production, credential, and non-idempotent write classes.
9. **FR-007-009:** Before any tool invocation, the gateway MUST intersect
   built-in, user, repository, session, role, and workflow allowlists so a
   lower-precedence grant cannot widen a higher-precedence deny or
   read-only restriction. Repository configuration MUST NOT widen user-global
   security policy.
10. **FR-007-010:** Role allowlists restrict which tool identifiers a routed
    role may call. Workflow step allowlists, when present, further restrict
    tools for that step. A tool absent from the effective allowlist is denied
    before transport and records a redacted policy denial.
11. **FR-007-011:** Effect-class protection MUST continue to force
    `approval-required` for paid inference, non-idempotent write, production,
    credential, and material-cost operations when policy would otherwise allow
    them. Operations configured with `approval: always` always require a human
    decision; `approval: never` is valid only when the effective intersected
    mode remains read-only and the effect class does not mandate approval.
12. **FR-007-012:** When approval is required, the gateway MUST emit
    `approval_requested`, transition the session to `awaiting_approval`, and
    MUST NOT start the external MCP call until `session.approval.resolve`
    records `grant` for that approval. A `deny` decision MUST leave a durable
    denial and MUST NOT invoke the server.
13. **FR-007-013:** Every gateway tool attempt MUST have a stable attempt
    identity and ordered facts compatible with the existing attempt model:
    planned (with effect class and operation), started when the external call
    begins, then a definite terminal success or failure, or `outcome_unknown`
    when the result is uncertain after start.
14. **FR-007-014:** Only an `idempotent-read` operation that also declares
    `idempotent: true` may be retried automatically, and only when failure is
    proven before `dispatch_started`. Mutating, production, credential, paid,
    and unclassified operations never retry automatically.
15. **FR-007-015:** Normalized tool events exposed to clients and durable
    history MUST include only bounded tool name, lifecycle category, redacted
    outcome, and correlation identifiers. Raw tool arguments, raw tool
    results, secret material, MCP protocol frames, environment values, and
    filesystem paths MUST NOT be persisted or returned on the public protocol.
16. **FR-007-016:** Cancellation of a session or attempt MUST cancel only
    gateway work belonging to that attempt. For stdio servers, the gateway
    MUST stop accepting new calls for the attempt, request cooperative cancel
    when the transport documents one, and otherwise terminate and reap the
    attempt-scoped child or in-flight call within the existing five-second
    public cancellation budget, reserving daemon finalization time. Lack of a
    definite cancelled terminal fact after start yields `outcome_unknown`.
17. **FR-007-017:** Daemon shutdown MUST reject new tool work, cancel or
    terminate active gateway calls, close HTTP clients, terminate supervised
    stdio children, drain pipes, and reap processes. A child that cannot be
    reaped is a startup or shutdown failure, not a successful tool outcome.
18. **FR-007-018:** Provider adapters remain free to keep provider-native
    tools that are not gateway-managed. This feature does not enable Claude,
    Codex, or Grok native write, shell, plugin, or provider-local MCP
    registration. Shared cross-provider capabilities MUST be exposed only as
    Workbench-managed MCP tools through the gateway.
19. **FR-007-019:** Default automated tests MUST use only repository offline
    fakes for stdio and HTTP MCP servers, make no network calls, discover no
    operator-installed MCP packages, read no credential store secrets, and
    consume no paid quota. Live validation, if present, MUST be ignored by
    default and opt-in.
20. **FR-007-020:** Public diagnostics MUST explain MCP availability, pin
    mismatches, policy denials, and approval waits without exposing secrets.
    CLI surfaces MAY extend existing `workbench config validate` and health
    reporting; they MUST NOT print credential values or raw tool payloads.

## Security Requirements

- **Data sensitivity/classification.** Tool arguments and results may contain
  confidential source, personal data, infrastructure inventory, or credentials.
  Only redacted lifecycle metadata enters durable history and public protocol
  payloads. Secret handles may be resolved at call time but never copied into
  manifests, locks, SQLite, WAL, exports, or logs.
- **Authentication/authorization.** The gateway is a same-user daemon surface,
  not a network multi-tenant API. Tool authority is granted by intersecting
  policy layers and explicit approvals. MCP server credentials stay in the OS
  keychain or provider-owned stores referenced by opaque handles.
- **Input validation.** Manifests, lock pins, executable paths, HTTP URLs,
  every MCP frame or HTTP body, tool names, effect classes, and size limits
  are untrusted. Validation occurs before state changes; allocation remains
  bounded (default 8 MiB encoded frame or response ceiling unless a tighter
  per-server limit is configured).
- **Cryptography in transit/at rest.** Non-loopback HTTP MCP uses TLS.
  Normalized sensitive session events retain existing per-session envelope
  encryption. MCP credentials are never stored in Workbench SQLite.
- **Logging/audit.** Audit records server identifier, transport class,
  lifecycle phase, attempt and correlation identifiers, policy decision source,
  approval identifiers, and stable outcomes. Raw arguments, results, tokens,
  URLs with secrets, and environment values are forbidden.
- **Error-handling information exposure.** Pin, policy, approval, transport,
  timeout, cancellation, and crash errors map to bounded categories and
  user-safe remediation. Child output and OS error strings never cross the
  client boundary unchanged.

## Acceptance Scenarios

1. **Pinned registry load:** Given a configuration and lock that pin one stdio
   and one HTTP MCP with matching digests, when the daemon starts, then both
   servers are available through the gateway and no provider launches them
   independently.
2. **Digest mismatch fails closed:** Given a lock pin for digest A, when the
   on-disk stdio artifact becomes digest B, then startup marks that server
   unavailable before any tool call and emits a redacted pin failure.
3. **Role allowlist deny:** Given role `reviewer` allows only tool `repo.read`,
   when a prompt routed to `reviewer` requests `cluster.mutate`, then the
   gateway denies the call before transport and records a policy denial.
4. **Workflow allowlist narrows role:** Given a role allows tools A and B but
   the active workflow step allows only A, when the step requests B, then B is
   denied before transport.
5. **Repository cannot widen user deny:** Given user-global policy denies tool
   `prod.deploy` and repository configuration grants it, when policy resolves,
   then the tool remains denied.
6. **Approval gate for mutation:** Given an allowed non-idempotent write tool
   with `approval: policy` or mandatory effect-class protection, when the tool
   is proposed, then `approval_requested` is recorded and the MCP call does not
   start until `grant`; `deny` prevents the call.
7. **Stdio supervision isolation:** Given two workspaces each using a fake
   stdio MCP, when one daemon stops, then only its children are reaped and the
   other workspace remains available.
8. **HTTP pin and bounds:** Given a fake HTTP MCP, when a response exceeds the
   encoded size ceiling or redirects to an unpinned host, then the call fails
   closed without treating partial mutation as success.
9. **Cancellation after start:** Given an in-flight mutating call after
   `dispatch_started`, when the session is cancelled without a definite
   cancelled terminal fact, then the attempt becomes `outcome_unknown` and is
   not retried automatically.
10. **Idempotent read retry bound:** Given an idempotent read that fails before
    `dispatch_started`, when the failure is proven pre-start, then a single
    automatic retry is allowed; the same failure after start is never
    auto-retried.
11. **Redacted audit and secrecy:** Given unique markers in tool arguments,
    results, environment, and credential handles, when success and failure
    paths run, then markers are absent from replies, telemetry, locks, SQLite,
    WAL, exports, and logs.
12. **Offline default suite:** Given default test configuration, when Feature
    007 tests run, then only committed fakes execute and no network,
    operator MCP install, credential store, or paid quota is used.
13. **Provider-native MCP remains blocked:** Given Claude, Codex, or Grok
    adapters from Features 004–006, when they run under this feature, then
    provider-local MCP registration and native mutation surfaces remain
    disabled; shared tools are reachable only through the gateway allowlist.
14. **Empty registry is valid:** Given no MCP servers are configured, when the
    daemon starts, then configuration and lock validation succeed and tool
    calls that require MCP servers fail as unavailable rather than crashing.
15. **Shutdown reaps children:** Given active stdio MCP children, when the
    daemon shuts down, then new calls are rejected, children are terminated
    and reaped, and incomplete work is not reported as successful completion.

## Observability

Extend existing policy and attempt instruments rather than inventing high-
cardinality series:

| Instrument | Notes |
|---|---|
| `workbench.policy.decision` | Continue `policy_result` in {allowed, approval-required, denied}; optional bounded `surface=mcp-gateway` only if cardinality stays fixed |
| `workbench.provider.duration` | Not used for MCP servers; prefer a bounded `workbench.tool.duration` histogram with labels `outcome` and `transport` in {stdio, http, fake} |
| Session attempt events | Reuse planned/started/terminal and approval event kinds with tool attempt IDs |

Logs report server id (configuration identifier only), transport class,
lifecycle phase, policy source, approval id, attempt id, correlation id, and
stable error category. Forbidden in every signal: tool arguments and results,
credential values, raw MCP frames, URLs with secrets, executable paths,
environment values, and prompt bodies.

Traces place gateway policy evaluation and MCP invocation as child spans of
the user input or control root span, carrying session and correlation
identifiers as attributes, never as metric labels.

## Clarifications

- Feature 001 already models MCP and tool registries, effect classes, and
  approval protocol without launching live servers. This feature activates
  lifecycle and enforcement for those contracts.
- Shared MCP is daemon-owned. Provider-native tools stay provider-specific
  until a later bridge feature deliberately exposes them under gateway policy.
- Default tool mode remains least privilege (`read-only` / deny-by-default for
  unlisted tools). Empty allowlists grant nothing.
- Package installation UX may remain operator-driven (place artifact, configure
  path, lock). Automatic unattended download from the public internet is not
  required for acceptance.
- Workflow executor sequencing is out of scope; this feature only enforces
  allowlists when a role or workflow step identity is already selected by the
  existing router.
- OpenRouter, VS Code control room, and Workbench ACP server remain separate
  issues and must not expand this feature's acceptance surface.
