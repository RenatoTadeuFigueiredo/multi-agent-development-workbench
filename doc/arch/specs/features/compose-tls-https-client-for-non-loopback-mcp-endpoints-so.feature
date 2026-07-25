Feature: Compose TLS HTTPS client for non-loopback MCP endpoints
  As a Workbench operator
  I want the MCP gateway to dial pinned non-loopback HTTPS endpoints over TLS
  So that remote tools stay governed without cleartext egress

  Scenario: Invoke pinned HTTPS MCP through an offline TLS fixture
    Given a pinned non-loopback https MCP endpoint and a local TLS fixture
    When the gateway invokes an allowed tool on that server
    Then the call completes over TLS
    And the public outcome is a redacted success

  Scenario: Reject cleartext non-loopback HTTP
    Given an http URL targeting a non-loopback host
    When the endpoint identity is parsed
    Then the configuration is rejected before transport

  Scenario: Reject unpinned HTTPS redirect
    Given a pinned https MCP endpoint
    When the response redirects to a different host
    Then the call fails closed with redirect rejection

  Scenario: Preserve loopback HTTP offline path
    Given a loopback http MCP fake
    When Feature 007 offline suites run
    Then the cleartext loopback path remains green

  Scenario: Redact secrets on TLS transport failure
    Given unique markers in tool arguments or headers
    When TLS handshake or transport fails
    Then the markers are absent from public errors audit locks and logs

  Scenario: Keep MCP free of heavy HTTP client crates
    Given the workbench-mcp package manifest
    When default acceptance inspects dependencies
    Then reqwest and hyper remain absent

  Scenario: Default suite stays offline
    Given default test configuration
    When Feature 013 tests run
    Then only offline fakes or the local TLS fixture execute
    And live public HTTPS remains ignored by default
