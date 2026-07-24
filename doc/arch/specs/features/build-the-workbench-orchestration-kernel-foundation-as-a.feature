Feature: Provider-independent orchestration kernel
  As a developer using multiple AI providers
  I want one durable and controllable orchestration session
  So that every client follows the same routing and policy decisions

  Scenario: Encrypted end-to-end fake-provider execution
    Given valid configuration and an available platform key store
    And an explicit route to a deterministic fake provider
    When a protocol client submits a prompt
    Then the input is persisted before provider dispatch
    And exactly one routing plan and provider attempt are recorded
    And planned, started, and terminal facts share that attempt ID
    And the fake provider completes the session
    And sensitive persisted payloads are encrypted

  Scenario: Configuration precedence and deterministic lock
    Given user, repository, and session layers define conflicting model aliases
    When the daemon resolves configuration twice
    Then the session value wins
    And the redacted snapshot records its source and content hash
    And the session lock links to the unchanged base lock
    And both session lock snapshots are byte-identical

  Scenario: Invalid higher-precedence configuration fails closed
    Given a session override contains an invalid provider reference
    And lower-precedence configuration contains a valid provider reference
    When the daemon resolves configuration
    Then configuration validation fails
    And no session is created
    And the lower-precedence value is not substituted

  Scenario: Explicit routing takes precedence
    Given a prompt explicitly targets the code-reviewer role
    And a coordinator adapter is available
    When the daemon routes the prompt
    Then exactly one routing plan selects the code-reviewer role
    And the coordinator adapter is not invoked

  Scenario: Low-confidence routing asks the user
    Given no explicit target, active workflow, or deterministic resolver matches
    And the fake coordinator returns confidence below the configured threshold
    When the daemon routes the prompt
    Then no executor receives the prompt
    And the session records a clarification request

  Scenario: Compatible provider fallback is visible before dispatch
    Given a role requires structured tool calls
    And its primary provider does not support structured tool calls
    And a compatible fallback provider is configured
    When provider preflight runs
    Then the fallback provider appears in the routing plan
    And dispatch uses the fallback only after the plan is emitted

  Scenario: Provider capability preflight rejects an invalid route
    Given a role requires structured tool calls
    And its resolved provider does not support structured tool calls
    And no compatible fallback is configured
    When provider preflight runs
    Then dispatch is rejected with capability_unavailable
    And the prompt is not sent to the provider

  Scenario: Attached clients share control events
    Given two protocol clients are attached to the same active session
    When one client pauses and resumes the session
    Then both clients observe the same ordered pause event
    And both clients observe the same ordered resume event
    And no new provider or tool action starts while paused

  Scenario: Redirect appends instruction without rewriting history
    Given two protocol clients are attached to the same paused session
    And the session has persisted prior instructions
    When one client redirects the session
    Then both clients observe the appended redirect instruction
    And the prior instructions remain byte-identical

  Scenario: A reconnecting client deduplicates replayed events
    Given session event 20 is the client's last durable cursor
    And event 21 was received but not checkpointed before disconnect
    And the session has persisted events 21 through 25
    When the client reconnects after event 20
    Then it uses stable event identifiers to deduplicate the replay
    And it retains one ordered copy of events 21 through 25

  @security
  Scenario: Protocol validation fails closed
    Given a client is unauthorized or requests an incompatible protocol major
    When the client attempts protocol negotiation
    Then the connection is rejected with a stable protocol error
    And no session state changes

  @security
  Scenario: An oversized frame is rejected
    Given an authorized client negotiated protocol version 1
    When it sends a frame larger than 8 MiB
    Then the frame is rejected with frame_too_large
    And no session state changes

  Scenario: A slow client cannot block the daemon
    Given two clients subscribe to one active session
    And one client accumulates 1024 queued events or 8 MiB
    When the daemon emits another event
    Then the slow client receives client_lagged and disconnects
    And the other client and session continue

  Scenario: Confirmed provider cancellation reaches cancelled
    Given an active session uses a fake provider that confirms cancellation
    When the user cancels the session
    Then cancellation is confirmed within five seconds
    And the session reaches the cancelled state
    And the prior event history remains readable

  Scenario: Unconfirmed cancellation requires human reconciliation
    Given an active session uses an unresponsive fake provider
    When the user cancels the session
    And no confirmation arrives within five seconds
    Then the session reaches outcome_unknown
    And no provider or tool attempt is retried automatically
    And explicit human reconciliation is required
    When the human reconciles with retry
    Then a new attempt is linked to the uncertain attempt

  @security
  Scenario: Global policy cannot be widened by repository configuration
    Given global policy denies a mutating tool
    And repository configuration grants that tool
    When session policy is resolved
    Then the mutating tool remains denied
    And the audit event identifies global policy as authoritative

  @security
  Scenario: A protected action waits for a recorded approval
    Given a production tool passes routing and capability preflight
    When the daemon proposes the protected action
    Then the session records an approval request
    And the production tool receives no call
    When a human grants that approval
    Then the decision and actor are recorded before the production tool starts
    When a human denies a later protected action
    Then the denial and actor are recorded
    And the session pauses without a second production tool call

  @security
  Scenario: Sensitive payloads are encrypted at rest
    Given a session contains a sensitive prompt and provider output
    When an inspector reads SQLite files and WAL pages without platform keys
    Then neither sensitive plaintext payload is recoverable
    And event metadata remains sufficient for authorized replay

  @security
  Scenario: Persistent mode requires a platform key store
    Given macOS Keychain or Linux Secret Service is unavailable
    When the client requests a persistent session
    Then creation fails with key_store_unavailable
    And no plaintext fallback database is created

  Scenario: Retention is disabled by default and configurable
    Given one terminal session has no retention period
    And another terminal session has a 30 day retention period
    When retention maintenance runs after 30 days
    Then the default-retention session remains readable
    And only the configured session enters the deletion state machine

  @security
  Scenario: Export and deletion protect retained history
    Given a retained encrypted session
    When the user exports and deletes it
    Then the portable export uses the age v1 encrypted format
    And the platform-stored session-key envelope and in-memory key are removed
    And a non-sensitive deletion tombstone is durable
    And remaining database pages cannot decrypt the deleted payload

  Scenario: Replaying a request cannot duplicate an accepted prompt
    Given a prompt command was accepted and its reply was lost
    When the client repeats the same request ID and parameters
    Then the recorded command result is returned
    And no second input or provider attempt is created
    When the client reuses that request ID with changed parameters
    Then the command fails with invalid_request

  Scenario: Default tests do not consume provider quota
    Given only default test configuration is present
    When the complete automated test suite runs
    Then only fake provider adapters are invoked
    And only an in-memory key store is invoked
    And no network request is attempted
