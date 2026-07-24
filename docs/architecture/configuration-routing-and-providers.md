# Configuration, Routing, and Provider Modularity

## Status

Accepted architecture. Features 001–004 implement the configuration
foundation, workspace-scoped sessions, thin VS Code bridge, and supervised
Grok ACP provider. Additional providers, shared MCP, and the complete
role-routing workflow remain gated by the project's Speckit process.

## Decision

Workbench routes roles, not vendor names. Workflows refer to stable roles,
roles select model aliases, and model aliases resolve through provider
adapters:

```text
Workflow -> Role -> Model alias -> Provider adapter -> Runtime model
```

Changing the model assigned to a role must not require changes to the workflow,
UI, session store, MCP gateway, or orchestration core. The core must use
capabilities and shared contracts; provider-specific conditionals are confined
to adapters.

## Configuration Layers

Configuration is resolved from lowest to highest precedence:

1. built-in schema and safe defaults;
2. user configuration in the platform configuration directory;
3. repository configuration in `.workbench/workbench.yaml`; and
4. explicit session overrides.

`.workbench/workbench.lock` is a deterministic required output for built-in,
user, and repository layers. It records the resolved configuration hash,
protocol version, adapter protocols and versions, executable digests, runtime
model identifiers, and MCP versions and checksums. A session override produces
a deterministic session lock linked to the base lock hash without rewriting
the repository lock; it cannot introduce an executable or MCP absent from that
base. Neither lock contains timestamps, environment-dependent paths, credential
values, or other sources of nondeterminism. Repository policy decides whether
the local base lock is tracked. Version-controlled files may contain credential
references such as portable `platform:openrouter`, but never secret values.
The daemon resolves `platform:` to macOS Keychain or Linux Secret Service.

Every session stores a redacted snapshot and hash of its resolved
configuration. Configuration changes apply to new sessions by default; an
active session changes only through an explicit, validated migration.

Feature 001 hashes recursively key-sorted, whitespace-free UTF-8 JSON with
BLAKE3-256. Later format changes require a lock schema version and explicit
migration.

## Declarative Model

The schema separates providers, model aliases, roles, routing, policies, and
workflows:

```yaml
version: 1

providers:
  codex:
    type: subscription-cli
  grok:
    type: acp
    executable: /absolute/canonical/path/to/grok
  openrouter:
    type: api
    credential_ref: platform:openrouter
    privacy:
      zero_data_retention: true
      data_collection: deny

models:
  coordinator:
    provider: codex
    runtime_model: gpt-5.6-sol
  implementation:
    provider: grok
    runtime_model: grok-4.5

roles:
  workspace-coordinator:
    model: coordinator
    tools: [repository, git, sessions]
  implementer:
    model: implementation
    tools: [repository, terminal, tests]

routing:
  default_role: workspace-coordinator
  confidence_threshold: 0.85

policies:
  default_tool_mode: read-only
  global_deny: []
  production_mutations: approval-required
```

This repository-layer example inherits omitted empty role fields and built-in
tool definitions from safe defaults. The post-merge resolved document is fully
explicit and validates against `workbench-configuration.schema.json`.

A later model migration normally changes only a model alias:

```yaml
models:
  implementation:
    provider: gemini
    runtime_model: gemini-next
```

Adding a model to an existing provider or a compatible generic API is
configuration-only. A provider with a new protocol requires an isolated
adapter implementing the shared contract.

## Message Routing

Every user message first reaches the daemon and is recorded before dispatch.
The router applies this order:

1. an explicit target such as `@codex` or a workflow command;
2. the active workflow, step, or attached session;
3. deterministic resolvers for status, history, and known data sources;
4. the configured coordinator model for natural-language classification; and
5. user clarification when confidence, scope, or authority is insufficient.

The router produces a visible plan containing the inferred intent, selected
role and model, required context, data sources, tools, risk, permissions, and
confidence. It never broadcasts a message to multiple providers implicitly.
Only the selected coordinator or executor receives the context required for
its task.

Typical mappings include:

| Request | Route |
|---|---|
| Explain an application rule | Code analyst with read-only repository tools |
| Show pending or recent work | Session, artifact, and Git resolvers |
| List open GitLab work | Project tracker with read-only GitLab MCP tools |
| Inspect production with Kubernetes | Operations analyst with approved read-only Kubernetes tools |

Repository configuration may narrow permissions but cannot widen the user's
global security policy. Mutating tools, ambiguous production targets, and
other material actions require the applicable approval gate.

## Provider Contract

Adapters implement a common lifecycle for capability discovery,
authentication status, session start and resume, prompt streaming,
cancellation, tool events, and normalized failures. Capabilities such as tool
calling, structured output, vision, context size, session resume, and ACP
support are discovered during preflight.

The MVP may compile first-party CLI adapters into the Rust workspace while
loading generic API and ACP-backed models from configuration. This keeps the
contract modular without making dynamic third-party code execution a release
requirement.

Removing a provider disables it for new routing, validates affected aliases and
fallbacks, and preserves redacted historical session metadata. Existing
sessions remain readable even when their original runtime is unavailable.

Every external operation reports whether it is idempotent, carries material
cost, or requires approval. Paid inference, mutation, production, credential,
and unknown-result operations never retry automatically. A started attempt
without a definite terminal event enters `outcome_unknown` and requires human
reconciliation.

The resolved configuration also contains central tool, data-source, and MCP
registries. Every data source references an idempotent read operation. Every
tool operation declares its effect class, idempotency, material-cost flag, and
approval mode. Semantic validation rejects missing role, model, tool,
data-source, MCP, fallback, and workflow-step references, and rejects
`idempotent: true` for paid inference, production access, credential access, or
non-idempotent mutations.

## Validation and Explainability

The CLI will expose stable diagnostics:

```bash
workbench config validate
workbench config explain
workbench providers probe
```

Validation rejects unresolved aliases, unsupported capabilities, missing
fallbacks, invalid tool grants, schema incompatibilities, and secret values in
version-controlled configuration. The UI must show the resolved destination
and permission scope before dispatch and retain the routing plan in the audit
log.
