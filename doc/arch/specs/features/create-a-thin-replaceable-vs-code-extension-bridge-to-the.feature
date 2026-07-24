Feature: Create A Thin Replaceable Vs Code Extension Bridge To The
  As a Workbench user
  I want to observe and control a local session from VS Code
  So that I can review agent work without leaving my editor

  Scenario: Attach replays and streams a local session
    Given a compatible local daemon and an existing session
    When the VS Code bridge attaches after its durable event cursor
    Then it renders each replayed and live event once in the Markdown preview

  Scenario: Prompt and controls use the local protocol
    Given the VS Code bridge is attached to a session
    When the user sends a prompt, pause, resume, cancel, or redirect command
    Then the bridge sends the versioned command to the daemon without provider credentials

  Scenario: A lost transport reconnects safely
    Given the VS Code bridge has observed a session event
    When the local socket closes and the daemon is available again
    Then it retries with bounded backoff from the last observed cursor

  Scenario: Markdown preview renders Mermaid
    Given a provider event contains a fenced Mermaid diagram
    When VS Code displays the virtual session document
    Then the native Markdown preview renders the diagram without a persisted transcript
