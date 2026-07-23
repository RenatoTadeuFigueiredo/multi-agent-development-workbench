# Configuration, Routing, and Provider Modularity

## Status

Accepted architecture for specification. Implementation remains gated by the
project's Speckit workflow.

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

`.workbench/workbench.lock` records resolved adapter versions, model
identifiers, MCP packages, checksums, and compatibility data when
reproducibility is required. Version-controlled files may contain credential
references such as `keychain:openrouter`, but never secret values.

Every session stores a redacted snapshot and hash of its resolved
configuration. Configuration changes apply to new sessions by default; an
active session changes only through an explicit, validated migration.

## Declarative Model

The schema separates providers, model aliases, roles, routing, policies, and
workflows:

```yaml
version: 1

providers:
  codex:
    type: subscription-cli
  grok:
    type: subscription-cli
  openrouter:
    type: api
    api_key: keychain:openrouter

models:
  coordinator:
    provider: codex
    model: gpt-5.6-sol
  implementation:
    provider: grok
    model: grok-4.5

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
  automatic_execution:
    read_only: true
    mutations: require_approval
```

A later model migration normally changes only a model alias:

```yaml
models:
  implementation:
    provider: gemini
    model: gemini-next
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
