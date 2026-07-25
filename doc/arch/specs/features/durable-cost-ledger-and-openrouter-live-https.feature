Feature: Durable cost ledger and OpenRouter live HTTPS
  As an operator
  I want session spend to survive daemon restart and optional live OpenRouter HTTPS
  So that budgets stay fail-closed without forcing network in CI

  Scenario: Persist redacted session spend across storage reopen
    Given a session with recorded paid-inference spend micros
    When encrypted storage is closed and reopened
    Then the prior spend micros are restored into the cost ledger

  Scenario: Budget deny uses restored spend
    Given restored spend at the session ceiling
    When a new attempt estimates additional cost
    Then the pre-dispatch budget gate denies the attempt

  Scenario: Default suite stays offline
    Given default test configuration
    When Feature 014 tests run
    Then only offline fakes execute and live HTTPS remains opt-in ignored

  Scenario: Live HTTPS client is composed behind explicit enablement
    Given the live OpenRouter transport constructor
    When operators enable live HTTPS with credentials
    Then the transport is not the offline fake
