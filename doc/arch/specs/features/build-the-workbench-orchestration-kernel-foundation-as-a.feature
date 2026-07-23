Feature: Provider-independent orchestration kernel
  As a developer using multiple AI providers
  I want one durable and controllable orchestration session
  So that every client follows the same routing and policy decisions

  Scenario: Explicit routing takes precedence
    Given a prompt explicitly targets the code-reviewer role
    And a coordinator adapter is available
    When the daemon routes the prompt
    Then exactly one routing plan selects the code-reviewer role
    And the coordinator adapter is not invoked

  Scenario: Low-confidence routing asks the user
    Given no explicit target, active workflow, or deterministic resolver matches
    And the fake coordinator returns confidence below the configured threshold
    When the daemon routes the prompt
    Then no executor receives the prompt
    And the session records a clarification request

  Scenario: Provider capability preflight blocks an invalid route
    Given a role requires structured tool calls
    And its resolved provider does not support structured tool calls
    And no compatible fallback is configured
    When provider preflight runs
    Then dispatch is rejected with a capability error
    And the prompt is not sent to the provider

  Scenario: Attached clients share control events
    Given VS Code and terminal clients are attached to the same active session
    When the terminal client pauses the session
    Then both clients observe the same ordered pause event
    And no new provider or tool action starts before resume

  Scenario: A reconnecting client replays missed events
    Given a client last observed session event 20
    And the session has persisted events 21 through 25
    When the client reconnects after event 20
    Then it receives events 21 through 25 once in sequence order

  Scenario: Global policy cannot be widened by repository configuration
    Given global policy denies a mutating tool
    And repository configuration grants that tool
    When session policy is resolved
    Then the mutating tool remains denied
    And the audit event identifies global policy as authoritative

  Scenario: Cancellation survives an unresponsive provider
    Given an active session uses an unresponsive fake provider
    When the user cancels the session
    Then the session reaches the cancelled terminal state
    And the prior session event history remains readable

  Scenario: Default tests do not consume provider quota
    Given only default test configuration is present
    When the complete automated test suite runs
    Then only fake provider adapters are invoked
    And no network request is attempted
