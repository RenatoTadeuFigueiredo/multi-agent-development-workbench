# Research: Claude Code Stream Adapter

## Confirmed Boundary

Claude Code 2.1.218 is installed on the reference macOS host. Its CLI documents
bidirectional `stream-json` input, newline-delimited structured output, partial
messages, model selection, session controls, permission modes, strict MCP
configuration, and disabled provider transcript persistence.

The official Python Agent SDK source at commit
`e6e07f1c9b0542217e1cf4913e96b161a6bf92b2` confirms the wire shapes used by
the SDK:

- user messages contain `type`, `message`, `parent_tool_use_id`, and
  `session_id`;
- SDK control requests contain `type`, `request_id`, and a typed `request`;
- initialization and interruption use correlated control requests;
- a result exposes `is_error`, `subtype`, `terminal_reason`, and session
  metadata; and
- `aborted_streaming` or `aborted_tools` proves an interrupted turn.

The SDK implementation is reference material only. Workbench does not vendor
or execute Python or TypeScript code.

## Authentication and Billing

Claude Code owns its login and stores credentials in the platform credential
store. `claude auth status --json` supplies a bounded preflight surface without
exposing tokens. Claude Code also gives API keys and alternate cloud providers
precedence in some configurations, so a subscription-only route must sanitize
those inherited selectors and reject non-subscription status.

Anthropic states that OAuth is for eligible Claude plans and native
applications, while developers building products that interact with Claude
should use API authentication and must not offer Claude login or route plan
credentials on behalf of users. As of 2026-07-24, programmatic `claude -p`
usage, Agent SDK use, and third-party application use continue to draw from
subscription limits. Anthropic paused the previously announced separate
monthly Agent SDK credit on 2026-06-15. Workbench therefore does not implement
login, promise subscription eligibility, or hide the billing mode. General
API-backed Claude use belongs in the OpenRouter or future Claude Console
provider.

## Update and Compatibility

Claude Code supports disabling background update checks with
`DISABLE_AUTOUPDATER=1`. Workbench additionally launches a private snapshot and
pins its exact version and digest, so an on-disk update cannot change an active
daemon. Compatibility remains capability-first after explicit re-locking.

The initial version floor is 2.1.214 because official release notes document
reliable draining of large structured output before process exit. The adapter
also requires interrupt-receipt support during initialization.

## Permission Decision

The current Workbench core cannot round-trip a provider-native permission
request through a durable mid-turn approval. This feature therefore exposes
only `Read`, `Glob`, and `Grep`, uses `dontAsk`, disables native persistence,
Chrome, skills, and MCP, and denies unknown control requests. Write, shell,
network, plugin, subagent, and centralized MCP authority remain separate
features.

## Primary Sources

- <https://code.claude.com/docs/en/cli-usage>
- <https://code.claude.com/docs/en/headless>
- <https://code.claude.com/docs/en/team>
- <https://code.claude.com/docs/en/legal-and-compliance>
- <https://code.claude.com/docs/en/permission-modes>
- <https://code.claude.com/docs/en/installation>
- <https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan>
- <https://github.com/anthropics/claude-agent-sdk-python/tree/e6e07f1c9b0542217e1cf4913e96b161a6bf92b2>
