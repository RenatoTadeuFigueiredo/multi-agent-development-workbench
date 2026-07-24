Feature: Supervised Claude Code subscription adapter
  As a Workbench operator
  I want Claude Code isolated behind the common provider contract
  So that Claude can inspect repositories without leaking credentials or bypassing policy

  Scenario: Execute through the offline fake
    Given a locked fake Claude executable with subscription authentication
    When a prompt is routed to the Claude provider
    Then the prompt streams once and completes without network or quota

  Scenario: Enforce the pinned launch profile
    Given a valid fake Claude executable
    When Workbench launches a prompt child
    Then the child observes the fixed safe flags and sanitized environment

  Scenario Outline: Reject an ineligible authentication mode
    Given the auth probe reports <auth_state>
    When provider preflight runs
    Then the provider is unavailable before prompt dispatch

    Examples:
      | auth_state          |
      | not logged in       |
      | API key             |
      | alternate provider  |

  Scenario: Reject executable replacement
    Given a lock created for one executable digest
    When the configured executable bytes change
    Then startup fails before a provider child is spawned

  Scenario: Correlate initialization
    Given control and stream frames arrive interleaved
    When the adapter initializes the child
    Then only the matching successful response and required capability unlock the prompt

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
      | an invalid envelope   |

  Scenario: Contain native tools
    Given Claude attempts a tool outside Read Glob and Grep
    When the attempt evaluates its available tool surface
    Then no protected action is available or approved

  Scenario: Normalize partial and final output
    Given the child emits partial text and a final assistant message
    When the result completes
    Then visible text is ordered once and one terminal completion is durable

  Scenario: Fail before external dispatch
    Given the child fails during probe or initialization
    When Workbench starts provider preflight
    Then no user message or external attempt is recorded

  Scenario: Preserve uncertainty after an active crash
    Given dispatch started is durable
    When the child exits before a definite result
    Then the attempt becomes outcome unknown without automatic retry

  Scenario: Confirm cancellation from the terminal result
    Given a prompt is active
    When interrupt succeeds and the result reports an aborted terminal reason
    Then the session becomes cancelled within five seconds

  Scenario Outline: Preserve uncertainty for unconfirmed cancellation
    Given a prompt is active
    When interrupt is followed by <unconfirmed_outcome>
    Then the child is reaped and the session becomes outcome unknown

    Examples:
      | unconfirmed_outcome       |
      | acknowledgment only       |
      | error result              |
      | silence                   |
      | end of stream             |
      | process crash             |

  Scenario: Contain sensitive process data
    Given unique markers exist in auth output environment stderr thinking and provider identifiers
    When success and failure paths run
    Then no marker appears in public or durable Workbench surfaces

  Scenario: Isolate workspace shutdown
    Given two workspace daemons own active fake children
    When one daemon shuts down
    Then only its children are reaped

  Scenario: Keep the default suite quota free
    Given the default automated configuration
    When the complete test suite runs
    Then only the committed fake executes without credentials network or quota
