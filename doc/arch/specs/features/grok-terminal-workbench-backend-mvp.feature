Feature: Grok terminal WorkbenchBackend MVP
  Selectable external ACP backend launch surface for workbench agent stdio.

  Scenario: WorkbenchBackend plans agent stdio with absolute paths
    Given an absolute workbench executable and workspace
    Then the launch plan is agent stdio in that workspace

  Scenario: Relative executable fails closed
    Given a relative workbench executable
    Then WorkbenchBackend construction fails closed

  Scenario: Compatibility pin surface exists
    Given the workbench-terminal-backend crate
    Then a Grok Build fork compatibility pin constant is published

  Scenario: Architecture residual is documented
    Given Feature 016 is delivered
    Then STATUS records the Grok pager fork residual outside this monorepo
