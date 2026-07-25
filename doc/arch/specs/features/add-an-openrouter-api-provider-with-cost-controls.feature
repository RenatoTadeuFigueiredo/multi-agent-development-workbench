Feature: OpenRouter API provider and cost controls
  As a Workbench operator
  I want OpenRouter isolated behind the common provider contract with budgets
  So that paid API models can run without leaking credentials or overspending

  Scenario: Execute through the offline fake
    Given a configured OpenRouter API provider with a resolvable credential and budget
    When a prompt is routed to the OpenRouter provider
    Then the prompt streams once and completes without network or quota

  Scenario: Reject a missing credential before dispatch
    Given the credential reference resolves to no secret
    When provider preflight or prompt setup runs
    Then the provider fails closed before any HTTP request

  Scenario: Reject an empty credential before dispatch
    Given the credential reference resolves to an empty secret
    When provider preflight or prompt setup runs
    Then the provider fails closed before any HTTP request

  Scenario: Enforce the session budget before dispatch
    Given the session spend already meets the configured max session budget
    When a new paid OpenRouter prompt is planned
    Then the attempt is denied before HTTP dispatch

  Scenario: Enforce the attempt budget before dispatch
    Given an attempt estimate that exceeds max attempt budget
    When a paid OpenRouter prompt is planned
    Then the attempt is denied before HTTP dispatch

  Scenario Outline: Enforce response body boundaries
    Given an encoded response body is <size>
    When the adapter reads the body
    Then the body is <outcome>

    Examples:
      | size                | outcome  |
      | exactly 8 MiB       | accepted |
      | one byte over 8 MiB | rejected |

  Scenario Outline: Reject malformed stream input
    Given the fake emits <malformed_input>
    When the adapter parses the stream
    Then it fails with a bounded redacted error

    Examples:
      | malformed_input |
      | invalid UTF-8   |
      | truncated SSE   |
      | invalid JSON    |

  Scenario: Normalize partial and final output
    Given the fake emits partial text deltas and a final completion
    When the result completes
    Then visible text is ordered once and one terminal completion is durable

  Scenario: Preserve uncertainty after mid-stream failure
    Given dispatch started is durable
    When the transport fails before a definite result
    Then the attempt becomes outcome unknown without automatic retry

  Scenario: Cancel an active stream
    Given an active OpenRouter prompt stream
    When cancellation is requested
    Then the stream stops within the cancellation budget

  Scenario: Keep secrets out of durable surfaces
    Given unique markers in the API key headers and response bodies
    When success and failure paths run
    Then the markers are absent from replies telemetry locks storage exports and logs

  Scenario: Default suite consumes zero credits
    Given default test configuration
    When all tests run
    Then only the offline fake executes and no public network keychain or OpenRouter quota is used

  Scenario: Require cost policy when API providers are configured
    Given a configuration with a type api provider and no cost policy
    When configuration validation runs
    Then validation fails closed
