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
- Unauthorized external actions performed by an agent.

General support questions and non-security bugs should use the public issue tracker.
