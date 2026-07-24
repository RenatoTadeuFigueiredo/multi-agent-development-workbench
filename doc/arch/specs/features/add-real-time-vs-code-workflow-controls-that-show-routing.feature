Feature: Add Real Time Vs Code Workflow Controls That Show Routing
  As a Workbench developer
  I want real-time workflow status and controls inside VS Code
  So that I can observe and steer multi-agent runs without leaving the editor

  Scenario: Routing plans and workflow transitions render in the session document
    Given an attached VS Code bridge receiving workflow events
    When routing_planned and workflow_transition events arrive
    Then the Markdown session document shows destination role, model, provider, step, iteration, and phase

  Scenario: Approval grant and deny use the versioned protocol
    Given an attached session with a pending approval_requested event
    When the user grants or denies the approval from VS Code
    Then the bridge sends session.approval.resolve with the approval id and decision

  Scenario: Lifecycle controls remain available during a workflow
    Given an attached workflow session
    When the user pauses, resumes, cancels, or redirects
    Then the bridge sends the matching session control command without provider credentials

  Scenario: Reattach after restart deduplicates durable events
    Given the bridge has observed workflow events and lost its transport
    When it reattaches from the durable sequence cursor
    Then previously rendered event identifiers are not shown twice

  Scenario: Offline suite stays free of network and credentials
    Given the deterministic fake protocol transport
    When the Feature 009 extension tests run
    Then they complete offline without provider credentials or paid quota
