Feature: Attach ACP agent stdio to running daemon
  As an editor or terminal ACP client
  I want workbench agent stdio to use the live local daemon
  So that sessions are shared with other Workbench clients

  Scenario: Attach initialize against a live local daemon
    Given a running workspace-local daemon with the offline fake provider
    When the ACP agent stdio backend attaches to the daemon socket
    And the client sends initialize with protocolVersion 1
    Then the agent responds with protocolVersion 1 and workbench identity

  Scenario: Create a durable session visible to other clients
    Given an ACP agent attached to the running daemon
    When the client sends session/new
    Then another local protocol client can list that session for the workspace

  Scenario: Prompt streams updates over the socket backend
    Given an ACP session on the attached daemon
    When the client sends session/prompt
    Then session/update content is observed and the prompt completes

  Scenario: Cancel an active prompt through the socket backend
    Given an active ACP prompt on the attached daemon
    When the client sends session/cancel
    Then the bridge requests daemon cancellation without panicking

  Scenario: Missing daemon fails closed
    Given no daemon socket at the discovered endpoint
    When the production agent stdio path attempts to attach
    Then the process fails closed with an actionable backend error

  Scenario: Default suite stays offline
    Given default test configuration
    When Feature 012 tests run
    Then only offline fakes and local daemon harnesses execute
