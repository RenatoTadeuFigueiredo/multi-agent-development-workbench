Feature: Versioned local session discovery
  As a user of a local Workbench daemon
  I want to discover sessions through a bounded metadata-only protocol command
  So that I can select a session without revealing its transcript

  Scenario: List a bounded metadata-only page
    Given the local daemon has persistent sessions
    When a compatible client sends session.list with limit 20
    Then it receives at most 20 summaries containing only session ID, state, creation time, and terminal time when present

  Scenario: Continue after an exclusive cursor
    Given a client has received a session summary with ID "018f47ef-9052-7b86-b31d-3f8962457777"
    When it sends session.list with that ID as before_session_id
    Then the returned page excludes that session and contains no repeated earlier summary

  Scenario: Select a session in VS Code
    Given a VS Code workspace resolves one local Workbench endpoint
    When the user chooses Workbench: Select Session
    Then the extension displays that endpoint's summaries in a Quick Pick and attaches the selected session

  Scenario: Create a session in VS Code
    Given a VS Code workspace resolves one local Workbench endpoint
    When the user chooses Workbench: New Session
    Then the extension creates a persistent session and attaches it
