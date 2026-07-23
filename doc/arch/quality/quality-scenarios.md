# Quality Scenarios

These ATAM-style scenarios define measurable system qualities using ISO/IEC
25010:2023 characteristic names.

The `Attribute` column uses the ISO/IEC 25010:2023 characteristic names exactly:
functional-suitability, performance-efficiency, compatibility,
interaction-capability, reliability, security, maintainability, flexibility, and
safety. `speckit validate` scans the table below (never the prose around it), so
keep it accurate and cover every attribute that matters for this system.

| ID | Attribute | Stimulus | Environment | Response | Measure |
|----|-----------|----------|-------------|----------|---------|
| QS-01 | functional-suitability | a developer submits a prompt to a configured workflow | normal local operation with a fake provider | the daemon emits one routing plan, executes one provider, and persists the outcome | all nine feature 001 acceptance scenarios pass |
| QS-02 | performance-efficiency | a deterministic route is requested | 100 active fake-provider sessions on a reference developer machine | the daemon records and emits the routing plan without a model call | p95 daemon routing latency is below 100 ms, excluding client transport |
| QS-03 | compatibility | a supported client or provider adapter negotiates its contract | current and previous supported protocol minor versions | compatible peers connect and incompatible majors fail clearly | every published protocol fixture and adapter contract test passes |
| QS-04 | interaction-capability | a first-time user attaches a second client and pauses work | a running local session with visible controls | both clients show the same pause and resume events | a usability test completes attach, pause, inspect, and resume without command documentation |
| QS-05 | reliability | a provider process stops responding during execution | an active session with persisted events | cancellation reaches a terminal state and prior history remains readable | terminal cancellation occurs within the configured deadline with zero lost committed events |
| QS-06 | security | repository configuration grants a globally denied mutating tool | policy resolution before dispatch | global denial wins and the decision is audited | every policy monotonicity property test passes and the tool receives zero calls |
| QS-07 | maintainability | a developer adds a provider using an existing protocol | the workspace under CI | only adapter registration, configuration, and provider tests change | core orchestration requires no vendor-specific branch and all gates pass |
| QS-08 | flexibility | an operator changes the model assigned to a role | a valid repository configuration and a new session | the new session resolves the new alias without workflow edits | configuration and lock data change while workflow files remain byte-identical |
| QS-09 | safety | an agent requests production mutation without approval | an active session with production tools configured | execution stops at an explicit approval event | no mutating tool event occurs before recorded human approval |
