Feature: Central MCP lifecycle and tool permissions
  As a Workbench operator
  I want the daemon to own shared MCP servers and tool permissions
  So that every provider uses the same pinned, allowlisted, and audited tools

  Scenario: Load pinned stdio and HTTP servers
    Given a configuration and lock that pin one stdio MCP and one HTTP MCP
    When the daemon starts the MCP gateway
    Then both servers are available through the gateway
    And no provider launches those servers independently

  Scenario: Reject digest mismatch before tool dispatch
    Given a lock pin for one stdio MCP digest
    When the on-disk artifact digest changes
    Then that server is unavailable before any tool call
    And clients receive only a redacted pin failure

  Scenario: Deny a tool absent from the role allowlist
    Given role reviewer allows only tool repo.read
    When a prompt routed to reviewer requests cluster.mutate
    Then the gateway denies the call before transport
    And a policy denial is recorded

  Scenario: Workflow allowlist narrows the role grant
    Given a role allows tools alpha and beta
    And the active workflow step allows only alpha
    When the step requests beta
    Then beta is denied before transport

  Scenario: Repository configuration cannot widen a user deny
    Given user-global policy denies tool prod.deploy
    And repository configuration grants tool prod.deploy
    When policy resolves for that tool
    Then the tool remains denied

  Scenario Outline: Gate protected operations on approval
    Given an allowed tool with effect class <effect_class>
    When the tool is proposed
    Then approval is required before the MCP call starts
    And a deny decision prevents the call

    Examples:
      | effect_class          |
      | non-idempotent-write  |
      | production            |
      | credential            |
      | paid-inference        |

  Scenario: Isolate supervised stdio children by workspace
    Given two workspaces each using a fake stdio MCP
    When one daemon stops
    Then only its MCP children are reaped
    And the other workspace remains available

  Scenario: Enforce HTTP pin and response bounds
    Given a fake HTTP MCP server
    When the response exceeds the encoded size ceiling or redirects to an unpinned host
    Then the call fails closed
    And partial mutation is not reported as success

  Scenario: Preserve uncertainty after cancel without a terminal fact
    Given an in-flight mutating tool call after dispatch started
    When the session is cancelled without a definite cancelled terminal fact
    Then the attempt becomes outcome unknown
    And the operation is not retried automatically

  Scenario: Allow pre-start retry only for idempotent reads
    Given an idempotent read fails before dispatch started
    When the gateway classifies the failure
    Then a single automatic retry is allowed
    And the same failure after start is never auto-retried

  Scenario: Redact secrets from audit surfaces
    Given unique markers in tool arguments results environment and credentials
    When success and failure paths run through the gateway
    Then the markers are absent from replies telemetry locks storage exports and logs

  Scenario: Default suite stays offline and quota-free
    Given default test configuration
    When Feature 007 tests run
    Then only committed MCP fakes execute
    And no network operator MCP install credential store or paid quota is used

  Scenario: Keep provider-native MCP registration disabled
    Given the supervised Grok Claude and Codex adapters
    When they run with the central gateway enabled
    Then provider-local MCP registration remains disabled
    And shared tools are reachable only through gateway allowlists

  Scenario: Accept an empty MCP registry
    Given no MCP servers are configured
    When the daemon validates configuration and starts
    Then validation succeeds
    And MCP-backed tools fail as unavailable without crashing

  Scenario: Reap children on shutdown
    Given active stdio MCP children
    When the daemon shuts down
    Then new tool calls are rejected
    And children are terminated and reaped
    And incomplete work is not reported as successful completion
