# Security Policy

## Supported Versions

The project is currently in design and pre-release development. Until the first release, security fixes apply only to the latest revision of `main`.

## Reporting a Vulnerability

Do not open a public issue for a suspected vulnerability.

Use GitHub's private vulnerability reporting feature from the repository's **Security** tab. Include:

- A clear description of the issue and affected component.
- Reproduction steps or a minimal proof of concept.
- Expected and observed behavior.
- Potential impact.
- Any suggested mitigation.

Avoid including real API keys, provider tokens, credentials, private source code, or personal data. Use redacted or synthetic values.

The maintainer will acknowledge a complete report, assess its severity, and coordinate remediation and disclosure. Timelines depend on impact and project maturity.

## Scope

Relevant reports include:

- Credential or provider-session exposure.
- Permission bypasses or unsafe tool execution.
- Sandbox, filesystem, or command-injection escapes.
- Cross-session data leakage.
- Secret persistence in logs, events, or artifacts.
- Supply-chain or update-channel compromise.
- Malicious or unauthenticated access to the local daemon, ACP bridge, or MCP
  gateway.
- Upstream terminal changes that bypass Workbench capabilities, approvals, or
  policy.
- Unauthorized external actions performed by an agent.

General support questions and non-security bugs should use the public issue tracker.

## Trust Boundaries

The Grok-derived terminal client is an unprivileged presentation client. It
must not read provider credentials or execute multi-provider workflows by
itself. It connects to `workbench agent stdio`, which authenticates to the
local daemon and exposes only negotiated ACP capabilities.

The official `grok` executable remains a separate provider process and owns its
subscription authentication. Workbench launches provider sessions with
automatic updates disabled for the lifetime of the workflow. Provider and
terminal updates are pinned, tested, reviewed, and promoted independently.

MCP packages, endpoints, credentials, and role-specific allowlists are managed
by the Workbench MCP gateway. Version-controlled manifests may reference
opaque `platform:` credential entries but must never contain secret values.
Feature 001 accepts no credentials through environment variables.

Repository configuration is untrusted input. It may narrow user policy but
cannot grant tools, credentials, environments, or mutation rights denied by a
higher-precedence security policy. Configuration parsing, alias resolution,
provider registration, capability claims, and session migration must fail
closed. Stored configuration snapshots and routing plans must be redacted
before persistence or export. Sensitive session and artifact payloads use
per-session envelope encryption; macOS Keychain or Linux Secret Service holds
the root key and wrapped session-key envelopes. Persistent operation fails
closed when same-user peer verification or the platform key store is
unavailable.

Upstream Grok Build synchronization is treated as a supply-chain change. The
mirror commit, downstream patch stack, licenses, build output, ACP contracts,
PTY behavior, and snapshots must be reviewed before release.
