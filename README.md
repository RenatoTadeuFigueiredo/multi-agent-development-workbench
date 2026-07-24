# Multi-Agent Development Workbench

![Claude, Codex, Grok, and OpenRouter connected to a portable Rust orchestration core](assets/readme-hero.svg)

<p align="center">
  <img alt="Project status: Claude and Grok provider adapters validated" src="https://img.shields.io/badge/status-Claude%20%2B%20Grok%20adapters-2563EB">
  <img alt="Core language: Rust" src="https://img.shields.io/badge/core-Rust-DEA584?logo=rust&logoColor=111827">
  <img alt="Primary interface: VS Code" src="https://img.shields.io/badge/interface-VS%20Code-007ACC?logo=visualstudiocode&logoColor=white">
  <img alt="License: Apache 2.0" src="https://img.shields.io/github/license/RenatoTadeuFigueiredo/multi-agent-development-workbench?color=2563EB">
  <img alt="Last commit" src="https://img.shields.io/github/last-commit/RenatoTadeuFigueiredo/multi-agent-development-workbench?color=475569">
</p>

<p align="center">
  <a href="README.md"><strong>English</strong></a>
  ·
  <a href="README.pt-BR.md">Português Brasileiro</a>
  ·
  <a href="CONTRIBUTING.md">Contributing</a>
  ·
  <a href="SECURITY.md">Security</a>
  ·
  <a href="LICENSE">License</a>
</p>

<p align="center">
  <strong>One workflow. The right model for every engineering role.</strong>
</p>

> [!IMPORTANT]
> Features 001–005 provide the Rust orchestration kernel, encrypted
> workspace-scoped sessions, headless CLI, thin VS Code bridge, session
> discovery, Grok Build over ACP v1, and a supervised read-only Claude Code
> subscription adapter. Codex, OpenRouter, shared MCP, the Workbench ACP
> server, and the Grok-derived TUI remain future increments.

## Table of Contents

- [Executive Summary](#executive-summary)
- [Proposed Experience](#proposed-experience)
- [Architecture](#architecture)
- [Configuration and Routing](#configuration-and-routing)
- [Project Readiness](#project-readiness)
- [Next Steps](#next-steps)
- [Speckit source of truth](doc/arch/functional/product-overview.md)

🧭 **Current phase:** Feature 005 implemented and validated locally.

## Executive Summary

The Multi-Agent Development Workbench is building one place to plan, execute,
review, and supervise software work performed by different AI coding agents.
The implemented foundation proves the provider-independent control plane with
deterministic fakes plus supervised Grok Build ACP and Claude Code stream-JSON
providers. Later adapters will add Codex and OpenRouter-backed models against
the same role-oriented contracts.

The primary interface is **Visual Studio Code**, using its agent sessions,
extension APIs, Git tooling, and native Markdown preview. The thin TypeScript
bridge connects to the provider-independent Rust core and can create, list,
select, attach to, and follow workspace-scoped sessions. A terminal client
derived from the open-source Grok Build pager will connect to the same daemon
through a narrow ACP bridge, while a future Workbench ACP server will let other
editors reuse the core.

The goal is not to create another AI model. It is to create a provider-independent control plane that makes multiple existing agents behave like one accountable engineering team.

## At a Glance

| One workspace | Role-based routing | Flexible access | Editor-portable |
|---|---|---|---|
| Prompts, progress, artifacts, diffs, and interventions stay together. | Claude specifies, Codex reviews, Grok implements, and workflows remain configurable. | Use native subscriptions or API models through OpenRouter. | Start in VS Code, continue in the terminal, and connect other editors through ACP. |

**Core principles:** provider independence · explicit handoffs · human control · durable sessions · auditable execution · specification-first delivery.

## Problem

Working with several AI tools currently requires separate windows, sessions, prompts, instruction files, and histories. This creates:

- Inconsistent rules between providers.
- Lost context during manual handoffs.
- Duplicate prompts and repeated repository discovery.
- Concurrent edits and unclear ownership.
- Limited visibility into progress, decisions, and failures.
- No reliable loop between specification, implementation, and validation.

## Proposed Experience

The user starts one session, describes the desired outcome, and selects or reuses a workflow. The orchestrator assigns each stage to the configured agent, streams progress into a unified timeline, persists artifacts, and moves work forward automatically.

```mermaid
flowchart LR
    U[User request] --> C[Claude: specification]
    C --> R[Codex: review and enrichment]
    R --> G[Grok: implementation]
    G --> V[Codex: tests and validation]
    V -->|Findings| G
    V -->|Approved| D[Completed change]
    C -. Optional review gate .-> U
    V -. Question or sensitive action .-> U
```

The user can interrupt, comment, pause, resume, or redirect the workflow at any time. Human approval gates are configurable by workflow; uncertainty, conflicting requirements, and sensitive operations always stop for confirmation.

## Interfaces

### VS Code Workbench

VS Code is the primary daily workspace:

- Built-in Chat and Agents surfaces for prompts, status, sessions, and interventions.
- Custom agents, subagents, and handoffs for role-oriented interactive work.
- Native Markdown preview with Mermaid rendering.
- Integrated source, Git, diffs, tests, debugging, and terminals.
- Optional isolated Git worktrees for concurrent tasks.

The implemented Workbench bridge uses stable public extension APIs and the
versioned local protocol for workspace endpoint resolution, session creation
and selection, attachment, reconnection, controls, and an in-memory Markdown
event document. The richer deterministic workflow view—provider stages,
conditional loops, approvals, and cross-provider audit—is a later UI
increment. Workflow and provider logic remain in Rust.

Specifications will be stored as normal repository files, for example:

```text
doc/arch/
├── sdd/001-feature-name/
│   ├── spec.md
│   ├── plan.md
│   └── tasks.md
├── adr/
├── schemas/
└── specs/features/
```

Reviewers can edit the documents directly or add visible review blocks:

```markdown
> [!REVIEW]
> Clarify the behavior when token rotation fails after the old token expires.
```

### Terminal Interface

The planned terminal application will reuse the mature Grok Build pager for prompt
editing, scrollback, Markdown and Mermaid, diffs, approvals, tasks, mouse
support, and terminal behavior. It will be built from the
[Workbench Grok Build fork](https://github.com/RenatoTadeuFigueiredo/grok-build)
and is intended to expose the same sessions, workflows, and policies as VS
Code:

```bash
workbench
workbench run workflows/feature.yaml
workbench status
workbench attach <session-id>
workbench pause <session-id>
workbench resume <session-id>
workbench cancel <session-id>
workbench daemon
workbench serve-acp
```

The planned terminal binary is a presentation client, not the orchestrator. It launches
`workbench agent stdio`, which translates ACP into the daemon's versioned local
protocol. The official `grok` executable remains a separate provider runtime
for SuperGrok-backed work. Interactive TUI and structured streaming output will
support local terminals, SSH, scripts, and CI jobs.

## Architecture

```mermaid
flowchart TB
    V[VS Code extension] -->|Versioned local protocol| D[Workbench daemon]
    T[Grok-derived Workbench TUI] -->|ACP stdio| B[Workbench terminal bridge]
    B -->|Versioned local protocol| D
    H[Headless CLI] --> D
    Z[Zed ACP client] --> X[ACP server]
    J[JetBrains ACP client] --> X
    X --> D
    D --> Q[Intent router]
    Q --> O[Orchestration Core]

    O --> P[Policy and permission broker]
    O --> S[Session and event store]
    O --> A[Artifact manager]
    O --> W[Workflow state machine]
    O --> M[Central MCP gateway]
    O --> C[Configuration and capability registry]

    W --> CA[Claude Code adapter]
    W --> CO[Codex CLI/ACP adapter]
    W --> GR[Grok Build ACP adapter]
    W --> OA[Generic API agent]

    CA --> CL[Claude subscription]
    CO --> OP[ChatGPT subscription]
    GR --> XA[SuperGrok subscription]
    OA --> OR[OpenRouter API]
```

All orchestration and application logic, plus every first-party binary, will be implemented in Rust as a small set of independently testable components. The thin VS Code client is the only planned runtime exception:

- **Workflow engine:** deterministic stages, transitions, policy-gated retries,
  and review loops.
- **Intent router:** explicit targets, contextual routing, deterministic queries,
  and coordinator-assisted classification without implicit broadcasts.
- **Configuration registry:** layered configuration, stable role and model
  aliases, capability preflight, lock data, and session snapshots.
- **Provider adapters:** process lifecycle, authentication detection, session resume, cancellation, and normalized events.
- **Policy broker:** shared instructions, tool permissions, and approval rules.
- **Event store:** encrypted durable session history and audit trail, initially
  backed by SQLite.
- **Artifact manager:** specifications, plans, decisions, diffs, and validation reports.
- **Editor bridge:** versioned local protocol between the VS Code extension and Rust daemon.
- **VS Code extension:** thin presentation and command adapter with no orchestration logic.
- **ACP server:** compatibility with Zed, JetBrains, and other ACP clients.
- **Terminal ACP bridge:** presents daemon sessions as a capability-negotiated
  ACP agent to the fork-derived pager.
- **Grok-derived terminal client:** reuses upstream presentation behavior
  without owning workflows, provider credentials, or policy.
- **MCP gateway:** installs, versions, supervises, filters, and audits shared
  MCP servers for every compatible provider.
- **Headless CLI:** portable scripted and CI access to the same core.
- **Generic agent runtime:** tool calling, context management, streaming, cost limits, and approval gates for API-backed models.

## Rust Implementation

The current implementation is a pinned Rust 1.95 Cargo workspace:

```text
crates/
├── workbench-core/          # Domain model, routing, policy, and ports
├── workbench-config/        # Layers, validation, snapshots, and locks
├── workbench-storage/       # Encrypted SQLite, key stores, export, retention
├── workbench-protocol/      # Strict versioned NDJSON commands and events
├── workbench-daemon/        # Application services and same-user Unix IPC
├── workbench-cli/           # Daemon lifecycle and headless client commands
├── workbench-acp/           # Bounded ACP v1 client and process supervision
├── workbench-claude/        # Claude stream-JSON and per-attempt supervision
└── workbench-testkit/       # Deterministic fakes, contracts, acceptance, SLOs
```

Build and exercise the current offline vertical slice:

```bash
make build
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- daemon
# in another terminal:
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
```

See the
[feature 001 quickstart](doc/arch/sdd/001-build-the-workbench-orchestration-kernel-foundation-as-a/quickstart.md)
for kernel prompts, event attachment, controls, and JSON output. The
[Grok ACP runbook](docs/operations/grok-acp-provider.md) covers the production
provider boundary. The
[Claude Code runbook](docs/operations/claude-code-provider.md) covers the
read-only subscription adapter. The terminal client, shared MCP gateway,
Workbench ACP server, and additional live providers are planned increments.

The VS Code extension is the only first-party component outside Rust because
VS Code extensions run in a TypeScript/JavaScript host. It remains a
replaceable client that renders daemon state and forwards commands. The
repository's bounded JSON-RPC/NDJSON ACP implementation is isolated in
`workbench-acp`, so protocol and provider changes do not leak into the domain
model.

The terminal presentation source is maintained separately in the Grok Build
fork. Its `main` branch mirrors upstream exactly; the `workbench` branch carries
the minimal external-backend patch stack. The main Workbench repository pins a
tested fork commit rather than vendoring the pager.

## Configuration and Routing

Providers, model aliases, roles, routing, policies, and workflows will be
declarative. Workflows target stable roles rather than vendor-specific models:

```yaml
version: 1

providers:
  claude:
    type: subscription-cli
    driver: claude-code
    executable: /absolute/canonical/path/to/versioned/claude
  grok:
    type: acp
    executable: /absolute/canonical/path/to/grok

models:
  specification:
    provider: claude
    runtime_model: fable
  implementation:
    provider: grok
    runtime_model: grok-4.5
  review-fallback:
    provider: grok
    runtime_model: grok-4.5

roles:
  workspace-coordinator:
    model: review-fallback
  product-architect:
    model: specification
  critical-reviewer:
    model: review-fallback
  implementer:
    model: implementation
  code-reviewer:
    model: review-fallback

routing:
  default_role: workspace-coordinator
  confidence_threshold: 0.85

policies:
  default_tool_mode: read-only
  global_deny: []
  production_mutations: approval-required

workflows:
  feature-delivery:
    steps:
      - id: specification
        role: product-architect
      - id: spec-review
        role: critical-reviewer
      - id: implementation
        role: implementer
      - id: validation
        role: code-reviewer
        on_findings: implementation
        max_iterations: 3
```

This is a repository-layer example. Safe built-ins supply omitted empty role
fields; tools and data sources must be declared before a role can reference
them. The resolved configuration is fully explicit and must satisfy the
committed schema. Codex and OpenRouter remain planned adapters and must not be
added as runnable providers until their drivers ship.

Configuration resolves from built-in safe defaults, user configuration,
`.workbench/workbench.yaml`, and explicit session overrides. A
deterministic `.workbench/workbench.lock` pins the non-session adapter, model,
MCP, and compatibility data. Session overrides create a linked session lock
without rewriting that base file. Secrets remain in the system keychain and
sensitive session payloads are encrypted with per-session keys.

Every free-form message reaches the daemon first. Explicit targets and active
workflow context take precedence, followed by deterministic status/history
resolvers and the configured coordinator. Before dispatch, the UI shows the
intent, role, resolved model, tools, data sources, permissions, and confidence.
Messages are never broadcast to multiple providers implicitly.

All providers implement a shared capability contract. Adding a model to an
existing adapter or compatible API is configuration-only; a new protocol
requires an isolated Rust adapter. Removing a provider validates affected
aliases and fallbacks without making historical sessions unreadable. Active
sessions retain a redacted configuration snapshot, so model changes affect new
sessions unless explicitly migrated.

Capability preflight distinguishes chat-only models from coding agents by
checking tool calling, structured output, context size, privacy, availability,
cost limits, resume support, and protocol compatibility. The full decision is
documented in
[`docs/architecture/configuration-routing-and-providers.md`](docs/architecture/configuration-routing-and-providers.md).

## Shared Rules and Context

The orchestrator will resolve a canonical instruction set for every stage:

1. Organization policies.
2. Repository `AGENTS.md`.
3. Provider-compatible repository instructions such as `CLAUDE.md`.
4. Workflow and role instructions.
5. The current user request.

The resolved instructions will be visible before execution and injected into every handoff. Provider-native configuration will remain supported, but conflicts will be reported instead of silently choosing one rule.

## Shared MCP and Tools

The daemon will own a canonical MCP manifest and lockfile. Installing or
updating a shared server once will pin its package or image version, checksum,
transport, credential reference, and policy. Compatible agents connect to a
single Workbench MCP gateway and receive role-specific tool allowlists.

Remote HTTP MCP servers naturally share one endpoint. Local stdio servers are
started and supervised by the gateway so provider clients do not independently
download or launch different versions. Credentials remain in the system
keychain or environment and are never written to the manifest.

Provider-native tools such as each agent's built-in file editor, shell, or
patch mechanism remain provider-specific. Capabilities that must behave
identically across Claude, Codex, Grok, and OpenRouter agents will be exposed as
Workbench-managed MCP tools. The gateway records calls, applies approval
policy, redacts secrets, and isolates sessions.

## Authentication and Billing

Native agents will use existing subscriptions; OpenRouter will be an explicit API-backed option:

| Agent or provider | Authentication path | Billing |
|---|---|---|
| Claude | Official Claude Code login | Provider-controlled; current `claude -p`, Agent SDK, and third-party app use draws from subscription limits |
| Codex | Codex login with ChatGPT | ChatGPT Pro |
| Grok | Grok Build browser/device login | SuperGrok Heavy |
| OpenRouter | API key stored in the system keychain | OpenRouter credits, billed per use |

Credentials remain owned by provider CLIs or the operating-system keychain and
must not be copied into workflow files or the session database. Anthropic
controls Claude subscription eligibility and billing; Workbench neither offers
Claude login nor promises that a plan covers programmatic use. The UI will
distinguish subscription routes from API usage and report OpenRouter token
consumption and cost per stage. Anthropic paused its announced separate Agent
SDK credit on June 15, 2026; operators must review the
[current plan guidance](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
before use.

The Grok-derived Workbench terminal does not reuse or access the Grok
subscription credential store. Only the unmodified official `grok` provider
process authenticates to SuperGrok.

VS Code's built-in third-party agent sessions and the providers' official extensions are different billing paths. The Workbench will default to the official Claude Code, Codex, and Grok Build authentication flows so an existing provider subscription is not silently replaced by GitHub Copilot billing. Copilot-backed sessions and VS Code BYOK models may still be used when explicitly selected.

## Editor Portability

VS Code is the first frontend, not the product foundation. Workflows, sessions, provider adapters, rules, and artifacts belong to the Rust core and remain usable without an editor.

- **VS Code:** uses the first-party extension and local editor bridge exposed by `workbench daemon`.
- **Terminal:** uses the Grok-derived Workbench pager through
  `workbench agent stdio`, with no editor dependency.
- **CI and scripts:** use the headless CLI and structured event output.
- **Zed:** connects to the optional `workbench serve-acp` compatibility endpoint.
- **JetBrains:** uses its ACP client and the same compatibility endpoint.

Editor-specific features such as diff presentation, panels, worktrees, and permission dialogs may look different. Capability negotiation will provide safe degradation, while Markdown artifacts and the event log remain the portable source of truth. No workflow, provider, credential, or policy logic may be implemented inside the VS Code extension.

## Safety and Change Control

- Only one workflow stage writes to a working tree at a time by default.
- Read-only reviewers cannot modify files unless the workflow grants permission.
- Commands, edits, approvals, and handoffs are recorded as events.
- Destructive filesystem actions, external publishing, and production mutations require explicit approval.
- Secrets are redacted from logs and never stored in artifacts.
- Cancellation propagates to provider processes without losing the resumable session.

## Performance Strategy

VS Code has a higher baseline resource cost than a native editor, but it removes the need to build an editor, agent-session shell, Git client, debugger, test UI, and Markdown renderer. The extension will activate lazily, reuse native VS Code UI where practical, and avoid a permanent webview process. The Rust daemon and provider processes will start on demand and remain persistent only while useful.

Concurrency limits, bounded event buffers, incremental artifact updates, and process health checks will prevent inactive agents from consuming resources indefinitely. Performance acceptance tests will measure extension activation, idle memory, event-stream responsiveness, and the complete multi-agent workflow rather than editor startup alone.

The terminal path avoids recreating a mature terminal framework. It reuses the
Grok pager's input, rendering, scrollback, and PTY behavior while keeping the
daemon and providers in separate processes. Terminal performance tests will
cover startup, memory, high-volume streaming, cancellation latency, and
reconnection.

## Open-Source Strategy

The orchestration engine will remain independent from any editor or model provider. A versioned local protocol is the primary boundary for the VS Code client, while ACP remains an interoperability boundary for compatible editors and agents. Both terminate at adapters around the same Rust application services.

The VS Code extension will use stable public extension APIs and remain independently replaceable. The MVP will not depend on private or proposed VS Code APIs for core workflow behavior.

[Grok Build](https://github.com/xai-org/grok-build) is Apache-2.0 licensed and
provides the terminal pager used as the Workbench TUI foundation. The
[Workbench fork](https://github.com/RenatoTadeuFigueiredo/grok-build) follows an
upstream-first patch-stack model:

- `main` is a fast-forward-only mirror of `xai-org/grok-build:main`;
- `workbench` contains a small, reviewed external ACP backend;
- product branches start from and target `workbench`;
- upstream updates are rebased on a temporary sync branch, reviewed with
  `git range-diff`, and never auto-merged; and
- release tags and tested upstream commits remain immutable.

The patch must preserve the original Grok backend and reuse its action, effect,
rendering, scrollback, permission, task, and testing architecture. Workflow
logic is not allowed in the fork. The complete decision, patch limits,
compatibility gates, and rollback policy are documented in
[`docs/architecture/grok-build-terminal-integration.md`](docs/architecture/grok-build-terminal-integration.md).

Feature 004 implements the required ACP v1 client subset as a small bounded
JSON-RPC/NDJSON adapter. The
[ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk) remains a
candidate for future ACP server and proxy surfaces, subject to compatibility
and supply-chain review. OpenRouter will be integrated through its HTTP API
directly from Rust rather than placing an agent runtime in the VS Code
extension.

This repository is licensed under the [Apache License 2.0](LICENSE). Reused or modified third-party components must retain their required copyright, license, and notice files.

## MVP Scope

The first usable release will include:

1. Claude, Codex, and Grok process adapters using provider-owned authentication.
2. A generic Rust agent runtime and OpenRouter adapter with capability and cost controls.
3. Sequential specification, review, implementation, and validation workflows.
4. Encrypted persistent sessions with pause, resume, cancel, and
   human-controlled reconciliation after uncertain outcomes.
5. A thin VS Code extension for prompts, workflow progress, artifacts, approvals, and session control.
6. A Grok-derived terminal client connected through the Workbench ACP bridge,
   plus headless JSON output.
7. Markdown artifacts, Mermaid diagrams, and configurable review gates.
8. Central instruction resolution, MCP lifecycle, tool permissions, and
   approval policies.
9. Layered configuration, explainable intent routing, model aliases, provider
   capability discovery, and safe provider removal.
10. Automated adapter, workflow, terminal compatibility, recovery, and
   end-to-end tests.

Parallel feature branches, remote workers, a visual workflow designer, team collaboration, analytics, deep integration with the VS Code Agents window, and additional editor-specific extensions are follow-up capabilities.

## Current Validation

- The nine-crate Rust workspace builds a same-user daemon and headless CLI
  with encrypted workspace isolation, deterministic fake execution, and a
  supervised Grok Build ACP v1 provider boundary.
- Sensitive session payloads are encrypted in SQLite; root keys use macOS
  Keychain or Linux Secret Service, and exports use age encryption.
- Feature 001 retains 23/23 repository-owned Rust acceptance contracts, and
  Feature 003 adds bounded workspace-local session discovery for the CLI and
  VS Code.
- Feature 004 targeted checks pass the ACP codec, supervisor, provider-port,
  launch, malformed-input, permission, cancellation, crash, and fake-process
  profiles. Its acceptance target reports 21 offline tests passed and one
  live-only test ignored by default. The 15 Gherkin headings expand into 23
  fingerprinted concrete cases whose 11 evidence tests cross the application,
  adapter, supervisor, transport, and fake-process layers.
- Feature 005 adds the isolated `workbench-claude` adapter, strict 8 MiB
  stream-JSON codec, subscription-only auth preflight, fixed read-only launch
  profile, per-attempt child ownership, correlated interruption, and 27/27
  fingerprinted quota-free acceptance cases.
- Speckit 0.18.10 verification is advisory for Features 001 and 004: it reports
  zero loaded bindings because its executable registry does not load their
  external Rust tests. The repository-owned runners are the authoritative
  acceptance gates.
- ACP boundary hardening accepts an exact 8 MiB frame with incremental newline
  scanning, rejects one byte over the limit, and divides cancellation into a
  4.5-second provider budget plus a 500-millisecond durable-finalization
  reserve.
- `make check` and `make supply-chain-ci` pass locally. PR #8 also records all
  four required jobs green in
  [GitHub Actions run 30106637866](https://github.com/RenatoTadeuFigueiredo/multi-agent-development-workbench/actions/runs/30106637866):
  macOS, Linux with Secret Service, supply chain, and VS Code.
- The production adapter launches the configured executable directly as
  `grok agent --no-leader stdio`, disables its auto-updater, pins its digest,
  negotiates ACP v1, and keeps Grok-owned authentication outside Workbench.
- The handshake-only production-path smoke passed on the recorded macOS host
  with Grok Build 0.2.111; it created no provider session, sent no prompt, and
  remains separate from the offline gate.
- The thin VS Code bridge creates, lists, selects, attaches to, and follows
  sessions only for its resolved workspace endpoint.
- Source inspection confirmed that the current pager spawns only its in-process
  GrokShell backend; the Workbench terminal backend remains a separate fork
  increment.
- Codex, OpenRouter, and shared MCP production adapters remain future features
  against the proven provider contract. Claude write tools and centralized
  permission/MCP bridging also remain out of scope.

## Success Criteria

The MVP is successful when a user can submit one feature request and:

- Follow every stage from a single VS Code Workbench conversation or terminal session.
- Inspect and comment on rendered specifications before or during execution.
- Automatically complete at least one implementation-review-fix loop.
- Apply the same repository rules to native and API-backed agents.
- Resume safely after an editor restart or provider failure.
- Distinguish subscription consumption from OpenRouter API cost for every stage.
- Open the same persisted session from VS Code and the terminal interface.
- Replace a role's model without changing its workflow, and explain every
  automatic routing decision before execution.
- Review a complete audit trail of prompts, decisions, commands, edits, and results.

## Project Readiness

| Foundation | Status | Owner |
|---|---|---|
| Product vision, architecture, and MVP boundary | Ready | This README |
| Public licensing and third-party notice policy | Ready | `LICENSE` and `NOTICE` |
| Contribution and vulnerability reporting policies | Ready | `CONTRIBUTING.md` and `SECURITY.md` |
| Shared repository instructions | Ready | `AGENTS.md` |
| Encoding, line endings, and GitHub collaboration templates | Ready | Repository configuration |
| Grok Build terminal integration decision and update policy | Ready | `docs/architecture/grok-build-terminal-integration.md` |
| Configuration, routing, and provider modularity decision | Ready | `docs/architecture/configuration-routing-and-providers.md` |
| Speckit scaffold, constitution, and governance baseline | Ready | `doc/arch/` |
| Orchestration kernel, encrypted sessions, protocol, and CLI | Implemented | Feature 001 |
| Thin VS Code session bridge | Implemented | Feature 002 |
| Workspace-local session discovery and isolation | Implemented | Feature 003 |
| Supervised Grok Build ACP provider | Implemented and validated | Feature 004 |
| Supervised read-only Claude Code provider | Implemented and validated locally | Feature 005 |
| Cargo workspace and deterministic acceptance harnesses | Implemented | Nine Rust crates |
| VS Code extension | Implemented foundation | TypeScript extension |
| Fork external-ACP backend and two-snapshot rebase spike | Pending | Later Speckit feature |
| Codex, OpenRouter, and shared MCP adapters | Pending | Later Speckit features |

The workspace pins Rust 1.95.0 and direct dependencies. The default
`make check` gate is deterministic and offline; real Keychain/Secret Service
coverage runs through the explicit `make test-platform` gate on macOS and
Linux.

## Specification-First Delivery

This README defines the product vision; `doc/arch/` defines implementation
requirements. Features 001–005 have completed the Speckit workflow through
`implement`. Feature 005 has complete local implementation evidence; pull
request evidence is recorded only after review:

```text
specify → clarify → plan → tasks → analyze → implement
```

Each phase produces reviewable Markdown artifacts, and `speckit validate` must
pass before a corpus or implementation commit. Future behavior changes require
a tracked change and a new or active Speckit feature before product code.

## Next Steps

1. Specify and implement Codex, OpenRouter, and shared MCP adapters
   independently against the proven provider contract.
2. Implement the Workbench ACP server, the terminal fork backend, and the
   richer workflow UI before piloting the complete multi-provider workflow.

## References

- [VS Code Agents](https://code.visualstudio.com/docs/agents/overview)
- [VS Code Custom Agents and Handoffs](https://code.visualstudio.com/docs/agent-customization/custom-agents)
- [VS Code Subagents](https://code.visualstudio.com/docs/agents/subagents)
- [VS Code Third-Party Agents](https://code.visualstudio.com/docs/agents/agent-types/third-party-agents)
- [VS Code Chat Participant API](https://code.visualstudio.com/api/extension-guides/ai/chat)
- [VS Code Language Models and BYOK](https://code.visualstudio.com/docs/agent-customization/language-models)
- [VS Code Markdown and Mermaid](https://code.visualstudio.com/docs/languages/markdown)
- [Agent Client Protocol](https://agentclientprotocol.com/)
- [ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk)
- [Grok Build documentation](https://docs.x.ai/build/overview)
- [Grok Build source](https://github.com/xai-org/grok-build)
- [Workbench Grok Build fork](https://github.com/RenatoTadeuFigueiredo/grok-build)
- [Grok Build terminal integration decision](docs/architecture/grok-build-terminal-integration.md)
- [Configuration, routing, and provider modularity decision](docs/architecture/configuration-routing-and-providers.md)
- [OpenRouter API](https://openrouter.ai/docs/quickstart)
- [Claude Code for VS Code](https://code.claude.com/docs/en/ide-integrations)
- [Codex authentication](https://learn.chatgpt.com/docs/auth)
- [Zed External Agents](https://zed.dev/docs/ai/external-agents)
- [JetBrains ACP support](https://blog.jetbrains.com/ai/2026/02/koog-x-acp-connect-an-agent-to-your-ide-and-more/)
