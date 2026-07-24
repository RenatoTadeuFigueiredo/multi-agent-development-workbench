Feature: Supervised Grok Build ACP provider adapter
  As a developer with a Grok Build subscription
  I want Workbench to supervise the external ACP runtime
  So that Grok participates in durable sessions without exposing credentials

  Scenario: Offline ACP prompt streams through one durable attempt
    Given a pinned fake ACP executable with protocol version 1
    And the fake reports Grok-owned authentication as available
    When Workbench routes a prompt to the ACP provider
    Then it initializes the child and creates an ACP session
    And normalized session updates retain one Workbench attempt ID
    And the prompt completes without network or real Grok execution

  Scenario: The fixed launch profile disables updates
    Given a valid pinned ACP executable
    When Workbench starts the provider child
    Then it directly passes "agent --no-leader stdio"
    And it sets "GROK_DISABLE_AUTOUPDATER" to "1"
    And it uses the canonical workspace as the working directory
    And no shell intermediary is used

  @security
  Scenario: An executable replacement fails before spawn
    Given the repository lock pins the configured ACP executable
    When that executable's digest changes
    Then adapter startup fails closed before a child is spawned
    And an explicit lock regeneration is required

  @security
  Scenario: ACP framing is strictly bounded
    Given an initialized full-duplex JSON-RPC 2.0 NDJSON child
    When a frame is exactly 8 MiB
    Then the frame is accepted
    When a frame exceeds 8 MiB by one byte
    Then it is rejected without an unbounded allocation

  @security
  Scenario Outline: Malformed ACP input fails closed
    Given an initialized fake ACP child
    When it emits <invalid_input>
    Then the adapter returns a stable redacted failure
    And raw child output is not logged

    Examples:
      | invalid_input          |
      | duplicate JSON keys    |
      | invalid UTF-8          |
      | truncated JSON         |
      | invalid JSON-RPC       |
      | an empty frame         |

  Scenario: A pre-dispatch child crash is definite
    Given the fake ACP child exits during initialize
    When provider preflight runs
    Then the adapter is unavailable
    And no prompt frame or provider attempt starts

  Scenario: An active child crash becomes uncertain
    Given dispatch_started is durable for an ACP prompt
    When the child exits before a terminal prompt response
    Then the attempt reaches outcome_unknown
    And no automatic retry or session resume occurs

  Scenario: Prompt cancellation is explicitly confirmed
    Given an active ACP prompt
    When Workbench sends session/cancel
    And the pending prompt returns stopReason cancelled within five seconds
    Then the session reaches cancelled

  Scenario Outline: Ambiguous cancellation requires reconciliation
    Given an active ACP prompt
    When Workbench sends session/cancel
    And the provider <ambiguous_result>
    Then the session reaches outcome_unknown within five seconds
    And explicit human reconciliation is required

    Examples:
      | ambiguous_result                              |
      | acknowledges cancel but leaves prompt running |
      | closes stdout                                 |
      | exits                                         |
      | returns an error                              |
      | completes without stopReason cancelled        |

  @security
  Scenario: Reverse permission is denied
    Given the ACP child sends a reverse permission request
    When the adapter handles the request
    Then it returns deny
    And no protected provider action is authorized

  @security
  Scenario: Authentication and diagnostics remain secret
    Given unique markers in Grok-owned authentication and child diagnostics
    When an ACP execution succeeds or fails
    Then the markers are absent from replies, logs, telemetry, locks, SQLite, WAL, and exports

  Scenario: A compatible additive update works after re-locking
    Given a same-major fake executable has been explicitly re-locked
    And it advertises all required capabilities plus unknown additive fields
    When Workbench initializes and prompts it
    Then the unknown additions are ignored safely
    And the baseline prompt completes

  Scenario: An incompatible update is unavailable
    Given a re-locked ACP executable reports an incompatible protocol or omits a required capability
    When provider preflight runs
    Then no prompt is dispatched
    And daemon startup returns a bounded incompatibility error
    And public adapter health remains unavailable

  Scenario: Workspace shutdown reaps only its child
    Given two workspaces supervise separate fake ACP children
    When one workspace daemon shuts down
    Then its child is terminated and reaped
    And the other workspace child remains available

  Scenario: The default suite consumes no provider quota
    Given only default automated test configuration
    When the complete gate runs
    Then every ACP child is the explicit fake executable
    And no installed grok executable, account, network, or paid model is used
