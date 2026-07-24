# Privacy Threat Model (LINDDUN)

The Workbench can process source code, prompts, provider account metadata, tool
results, and audit history that identify developers or people represented in a
repository. This LINDDUN model covers every category using the required
lowercase tokens.

| ID | Category | Threat | Affected Data | Mitigation |
|----|----------|--------|---------------|------------|
| PT-01 | linking | session, Git, tool, and provider events are correlated into a broader developer activity profile | session identifiers, repository metadata, routing history | scope identifiers to one local store, exclude them from metric labels, and make exports explicit |
| PT-02 | identifying | prompts or artifacts reveal a developer or data subject to a provider that did not need the data | names, emails, source code, issue content, provider account metadata | show destination and context before dispatch, minimize context by role, and redact persisted snapshots |
| PT-03 | non-repudiation | durable audit history proves a person approved or initiated sensitive work beyond the required purpose | approval actor, timestamps, tool decisions, session history | retain history until explicit deletion by default, support configured retention, and require encrypted export |
| PT-04 | detecting | another local process infers that a person, project, or provider session exists | IPC endpoints, process list, state paths, timing | use same-user IPC permissions, opaque identifiers, private state directories, and no network listener by default |
| PT-05 | data-disclosure | credentials, prompts, source, model output, or tool results leak through storage, logs, errors, clients, or providers | keychain references, prompt bodies, artifacts, SQLite pages, WAL files, provider diagnostics | never persist credential values, envelope-encrypt sensitive payloads, require same-user clients, log metadata by default, and redact errors |
| PT-06 | unawareness | a user does not realize which model, tools, context, or external service will receive a prompt | routing plan, context sources, tool grants, provider privacy mode | display the routing plan and approvals before dispatch and retain the decision in session history |
| PT-07 | non-compliance | retained prompts or cross-provider transfers exceed project policy or provider privacy commitments | event database, wrapped keys, artifacts, exports, lock files, provider requests | pin resolved policy in the deterministic lock, validate retention rules, use age-encrypted exports, and audit every external dispatch |
| PT-08 | data-disclosure | the supervised ACP child exposes confidential content through raw protocol frames, diagnostics, process metadata, or provider session identifiers | prompts, model output, repository paths, stdout, stderr, JSON-RPC bodies, environment, provider session IDs | use bounded inherited stdio, persist only normalized encrypted events, discard or bound diagnostics, and exclude raw child data and identifiers from logs, telemetry, replies, and exports |
| PT-09 | unawareness | Grok-owned plugins, hooks, skills, MCPs, or account configuration change provider behavior outside Workbench policy | provider configuration, tool behavior, repository data, external requests | launch with `--no-leader`, disclose that supervision is not an OS sandbox, require operator review of native configuration, advertise no client filesystem or terminal authority, and deny every reverse permission request |
| PT-10 | non-compliance | an automatic or unreviewed executable update changes protocol, privacy, permissions, or data handling during work | executable, adapter version, protocol capabilities, session traffic | disable the supervised auto-updater, pin version and SHA-256, require explicit update and re-lock, negotiate required capabilities, and fail closed before spawn on mismatch |
| PT-11 | unawareness | inherited API credentials or cloud selectors silently change a configured Claude subscription route into API billing | environment, endpoint selection, account state, billing mode | require subscription auth preflight, remove API-key, alternate-endpoint, and cloud-provider selectors, expose bounded unavailability, and document provider-controlled billing |
| PT-12 | data-disclosure | Claude stream frames, thinking, usage, tool arguments, stderr, or provider identifiers escape the normalized encrypted event boundary | source, prompts, model output, tool data, account and session metadata | enforce strict bounded NDJSON, discard stderr and private fields, expose only visible text and bounded read-tool names, and prohibit raw frames from logs, telemetry, replies, locks, and exports |
| PT-13 | non-compliance | Claude-native shell, write, web, MCP, skill, plugin, browser, or subagent authority bypasses centralized policy | repository, local system, external services, provider configuration | use safe mode, strict empty MCP, disabled slash commands and Chrome, `dontAsk`, no persistence, and an exact `Read`, `Glob`, `Grep` tool set; fail closed on any other tool or control request |

## Supervised Grok ACP Child Boundary

The official Grok Build executable is a separate same-user process and an
untrusted provider boundary. Workbench invokes the canonical pinned file
directly as `grok agent --no-leader stdio`, sets
`GROK_DISABLE_AUTOUPDATER=1`, and gives it the canonical workspace as its
working directory. There is no shell or network listener, but this feature
does not provide an operating-system sandbox or prevent provider-owned network
access.

Grok Build exclusively owns login, refresh, cookies, tokens, and their storage.
Workbench must not inspect those values, even for health, debugging, or update
evidence. The adapter receives only the bounded authentication state needed for
startup. Public adapter health exposes only `available` or `unavailable`;
authentication, compatibility, spawn, and crash causes remain stable redacted
errors.

Both directions of ACP stdio are untrusted and independently serviced. The
adapter rejects malformed or oversized frames before state changes, denies
reverse permission requests, and never grants authority from additive unknown
fields. Raw stdout, stderr, environment values, JSON-RPC payloads, repository
paths, and provider session identifiers are prohibited from every log,
telemetry signal, client error, and support artifact.

Executable pinning prevents silent substitution but does not establish vendor
trust. Operators must review distribution provenance and Grok-owned extension
configuration before re-locking. The
[supply-chain review](../../../docs/security/grok-acp-supply-chain-review.md)
records the update controls and residual same-user process risk.

## Supervised Claude Code Child Boundary

Feature 005 invokes an operator-installed official Claude Code executable as a
fresh same-user child for each attempt. Lock generation and startup use a
private executable snapshot, pin `claude-code-stream-json/1`, version, and
SHA-256, and set `DISABLE_AUTOUPDATER=1`. Workbench never discovers the binary
through `PATH`, runs login or update commands, reads the credential store, or
receives an OAuth token.

Authentication preflight accepts only an already authenticated Claude
subscription status. API keys, auth tokens, alternate endpoints, and
Bedrock/Vertex/Foundry selectors are removed from the child environment so a
subscription route cannot silently change billing paths. Anthropic controls
programmatic-use eligibility and charging; the adapter does not make a billing
guarantee or act as a credential broker.

The direct launch uses bidirectional stream JSON, safe mode, disabled Chrome,
slash commands, and provider transcript persistence, a strict empty MCP
manifest, `dontAsk`, and only `Read`, `Glob`, and `Grep`. This is authority
reduction, not an operating-system sandbox. The child can still read the
canonical workspace and contact provider services as the current user.

Every frame is untrusted, UTF-8, newline-delimited, duplicate-key-free, and
limited to 8 MiB. Only normalized visible text and bounded read-tool names may
enter encrypted session history. Thinking, usage, raw tool data, auth fields,
stderr, protocol bodies, executable and workspace paths, and provider
identifiers are discarded. Cancellation is confirmed only by a correlated
successful interrupt response followed by `aborted_streaming` or
`aborted_tools`; all other post-dispatch loss remains `outcome_unknown`.
