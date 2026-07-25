Feature: Provider-native write tools under central policy
  Fail-closed Claude/Codex native writes gated by allowlist and approval mode.

  Scenario: Default policy keeps native writes disabled
    Given the default Workbench configuration
    Then provider-native writes are disabled for every provider

  Scenario: Allowlist enables Claude Write under policy
    Given provider_native_writes mode approval-required
    And the claude provider is on the allowlist
    Then Claude Write tools are accepted by the write-enabled protocol

  Scenario: Deny path rejects write tools without allowlist
    Given provider_native_writes mode disabled
    Then Claude Write tools fail closed
    And Codex file_change items fail closed

  Scenario: Shared tools remain on the MCP gateway
    Given provider-native writes are configured
    Then shared tools still route through the central MCP gateway policy
