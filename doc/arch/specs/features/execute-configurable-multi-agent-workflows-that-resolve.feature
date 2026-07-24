Feature: Configurable multi-agent workflow executor
  As a workflow author and developer
  I want durable multi-stage workflows over provider-neutral roles
  So that specification, review, implementation, and validation advance with bounded correction and recovery

  Scenario: Validate well-formed and broken workflow graphs
    Given a configuration with a multi-step workflow binding known roles
    And a second configuration that references a missing role
    When configuration validation runs
    Then the well-formed workflow is accepted
    And the broken workflow fails closed before lock generation

  Scenario: Advance sequential stages with explainable routing plans
    Given a three-step workflow and offline fake providers
    When the workflow run starts
    Then each step dispatches in declaration order
    And every dispatch emits a routing plan with rule workflow
    And the run completes after the last successful step

  Scenario: Prove the primary Claude to Codex to Grok to Codex path offline
    Given roles bound to Claude Codex Grok and Codex model aliases
    And only offline fake adapters
    When the primary multi-provider workflow runs
    Then four attempts complete in Claude then Codex then Grok then Codex order
    And default tests use zero network and zero paid quota

  Scenario: Bound review-correction loops
    Given a review step with on_findings and max_iterations 2
    And the review fake keeps reporting findings
    When the automatic correction loop runs
    Then at most two correction iterations dispatch
    And the run pauses awaiting human decision afterward

  Scenario: Select configured fallback when primary preflight fails
    Given a step whose primary provider is unavailable
    And a compatible fallback model alias
    When the step is entered
    Then the routing plan selects the fallback
    And the selection reason is durable and explainable

  Scenario: Pause and resume freeze and continue advancement
    Given an active workflow run between steps
    When the user pauses the session
    Then no further step dispatch occurs while paused
    When the user resumes
    Then the next step continues from the durable active step

  Scenario: Cancel terminates the run without inventing success
    Given an active workflow step attempt
    When the user cancels
    Then attempt cancel semantics apply
    And the workflow run becomes cancelled or outcome_unknown without silent success

  Scenario: Redirect injects instruction without rewriting history
    Given a workflow run that can accept redirect
    When the user redirects with additional instruction
    Then session history is not rewritten
    And the next dispatch includes the redirect instruction

  Scenario: Recover active step after daemon interruption
    Given a durable mid-workflow run
    When the daemon restarts and reloads the session
    Then the active step and phase match the last durable facts
    And uncertain attempts remain outcome_unknown until reconciliation

  Scenario: Workflow step tools stay under the central MCP gateway
    Given a step tool allowlist that excludes a mutating tool
    When the step requests the excluded tool
    Then the central gateway denies before transport

  Scenario: Default suite stays offline and quota-free
    Given default test configuration
    When Feature 008 acceptance runs
    Then no live provider quota is consumed
    And no external network is required
