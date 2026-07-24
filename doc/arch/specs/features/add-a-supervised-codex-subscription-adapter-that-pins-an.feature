Feature: Supervised Codex subscription adapter
  As a Workbench operator
  I want Codex isolated behind the common provider contract
  So that Codex can review repositories without leaking credentials or bypassing policy

  Scenario: Execute through the offline fake
    Given a locked fake Codex executable with ChatGPT subscription authentication
    When a prompt is routed to the Codex provider
    Then the prompt streams once and completes without network or quota

  Scenario: Enforce the pinned launch profile
    Given a valid fake Codex executable
    When Workbench launches a prompt child
    Then the child observes exec json ephemeral read-only sandbox and sanitized environment

  Scenario Outline: Reject an ineligible authentication mode
    Given the auth probe reports <auth_state>
    When provider preflight runs
    Then the provider is unavailable before prompt dispatch

    Examples:
      | auth_state          |
      | not logged in       |
      | API key             |
      | unknown auth mode   |

  Scenario: Reject executable replacement
    Given a lock created for one executable digest
    When the configured executable bytes change
    Then startup fails before a provider child is spawned

  Scenario Outline: Enforce frame boundaries
    Given an encoded stream frame is <size>
    When the adapter reads the frame
    Then the frame is <outcome>

    Examples:
      | size                  | outcome  |
      | exactly 8 MiB         | accepted |
      | one byte over 8 MiB   | rejected |

  Scenario Outline: Reject malformed stream input
    Given the child emits <malformed_input>
    When the adapter parses the stream
    Then it fails with a bounded redacted error

    Examples:
      | malformed_input       |
      | duplicate keys        |
      | invalid UTF-8         |
      | truncated JSON        |
      | an empty frame        |
      | an invalid event      |

  Scenario: Contain sandbox and elevated tools
    Given Codex attempts workspace-write danger-full-access MCP plugins or approval bypass
    When the launch profile is applied
    Then no protected mutation is available or approved

  Scenario: Normalize partial and final output
    Given the child emits partial text and a final assistant message
    When the result completes
    Then visible text is ordered once and one terminal completion is durable

  Scenario: Fail before external dispatch
    Given the child fails during probe or launch
    When Workbench starts provider preflight
    Then no successful external attempt is claimed

  Scenario: Preserve uncertainty after an active crash
    Given dispatch started is durable
    When the child exits before a definite result
    Then the attempt becomes outcome unknown without automatic retry

  Scenario: Confirm cancellation from a terminal abort event
    Given an active prompt
    When a documented abort terminal event is observed before reaping
    Then the session reaches cancelled within five seconds

  Scenario: Leave cancellation unconfirmed without a terminal abort
    Given an active prompt without a confirming abort event
    When the provider cancellation budget expires
    Then the child is reaped and the session reaches outcome unknown

  Scenario: Keep secrets out of durable surfaces
    Given unique markers in auth output environment stderr tool data and provider ids
    When success and failure paths run
    Then the markers are absent from replies telemetry locks storage exports and logs

  Scenario: Isolate workspace shutdown
    Given two daemons with active fake children
    When one daemon stops
    Then only its children are reaped and the other workspace remains available

  Scenario: Default suite consumes zero quota
    Given default test configuration
    When all tests run
    Then only the committed fake executes and no installed Codex binary credential store network or quota is used

  Scenario: Never open operator credential files
    Given CODEX_HOME contains auth material
    When Workbench probes and runs the adapter
    Then credential files are never opened copied or logged
