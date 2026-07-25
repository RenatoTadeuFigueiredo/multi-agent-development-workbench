Feature: Workbench ACP server and terminal client
  As an editor or terminal client
  I want Workbench sessions over ACP stdio
  So that I can reuse the daemon without embedding providers

  Scenario: Initialize the Workbench ACP agent offline
    Given an offline Workbench ACP agent stdio process
    When the client sends initialize with protocolVersion 1
    Then the agent responds with protocolVersion 1 and workbench identity

  Scenario: Create a session through the bridge
    Given an initialized offline Workbench ACP agent
    When the client sends session/new
    Then the agent returns a non-empty session id backed by the daemon

  Scenario: Prompt streams assistant updates
    Given an offline ACP session on the fake provider path
    When the client sends session/prompt
    Then session/update content is observed and the prompt completes

  Scenario: Cancel an active prompt
    Given an active offline ACP prompt
    When the client sends session/cancel
    Then the bridge requests daemon cancellation without panicking

  Scenario: Reject oversized frames
    Given an encoded ACP frame one byte over 8 MiB
    When the agent reads the frame
    Then the frame is rejected fail-closed

  Scenario: Default suite stays offline
    Given default test configuration
    When Feature 011 tests run
    Then only offline fakes and local daemon harnesses execute
