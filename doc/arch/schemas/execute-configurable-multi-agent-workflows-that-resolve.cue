// DDD role: ValueObject

package schemas

// #ExecuteConfigurableMultiAgentWorkflowsThatResolve models the workflow
// executor contracts for Feature 008.
// DDD role: ValueObject
#ExecuteConfigurableMultiAgentWorkflowsThatResolve: {
	executorOwner: "workbench-daemon"
	graph:         #WorkflowGraphPolicy
	runtime:       #WorkflowRuntimePolicy
	recovery:      #WorkflowRecoveryPolicy
	testing:       #WorkflowTestingPolicy
}

// DDD role: ValueObject
#WorkflowGraphPolicy: {
	structure:  #WorkflowGraphStructure
	bounds:     #WorkflowGraphBounds
	validation: #WorkflowGraphValidation
}

// DDD role: ValueObject
#WorkflowGraphStructure: {
	sequentialSteps:          true
	correctionEdge:           true
	fallbackModelAliases:     true
	freeFormDagOutOfScope:    true
	parallelFanOutOutOfScope: true
}

// DDD role: ValueObject
#WorkflowGraphBounds: {
	maxIterationsCeiling: 8
	defaultMaxIterations: 1
}

// DDD role: ValueObject
#WorkflowGraphValidation: {
	roleMustExist:       true
	onFindingsMustExist: true
	emptyStepsInvalid:   true
}

// DDD role: ValueObject
#WorkflowRuntimePolicy: {
	selectedRule:              "workflow"
	routingPlanBeforeDispatch: true
	orchestratorAttempts:      true
	gatewayToolsOnly:          true
	lifecycle:                 #WorkflowLifecycle
}

// DDD role: ValueObject
#WorkflowLifecycle: {
	phases:   #WorkflowClosedSet
	controls: #WorkflowClosedSet
}

// DDD role: ValueObject
#WorkflowClosedSet: {
	// Closed string sets for phases or controls (values filled by policy docs).
	kind:   "phases" | "controls"
	values: [...string] & [_, ...]
}

// DDD role: ValueObject
#WorkflowRecoveryPolicy: {
	rebuildFromDurableEvents: true
	neverInventSuccess:       true
	preserveOutcomeUnknown:   true
}

// DDD role: ValueObject
#WorkflowTestingPolicy: {
	offlineFakesOnly:   true
	zeroNetworkDefault: true
	zeroQuotaDefault:   true
	primaryPath:        "claude-codex-grok-codex"
	liveProvidersOptIn: true
}
