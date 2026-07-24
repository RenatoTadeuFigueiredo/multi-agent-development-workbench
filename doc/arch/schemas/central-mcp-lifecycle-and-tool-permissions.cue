// DDD role: ValueObject

package schemas

// #CentralMcpLifecycleAndToolPermissions models the daemon-owned MCP gateway.
// DDD role: ValueObject
#CentralMcpLifecycleAndToolPermissions: {
	gatewayOwner: "workbench-daemon"
	lock: #McpLockPolicy
	runtime: #McpRuntimePolicy
	policy: #McpAccessPolicy
	testing: #McpTestingPolicy
}

// DDD role: ValueObject
#McpLockPolicy: {
	pinVersion: true
	pinSha256: true
	autoUpdate: false
	emptyRegistryValid: true
	stdioTransport: true
	httpTransport: true
}

// DDD role: ValueObject
#McpRuntimePolicy: {
	stdioLaunch: "direct-argv"
	stdioShell: false
	workspaceIsolation: true
	httpNonLoopbackTLS: true
	rejectUnpinnedRedirect: true
	maxEncodedResponseBytes: 8388608
}

// DDD role: ValueObject
#McpAccessPolicy: {
	repositoryCannotWidenUser: true
	defaultUnlisted: "denied"
	approvalProtocol: "session.approval.resolve"
	persistRawArguments: false
	persistRawResults: false
}

// DDD role: ValueObject
#McpTestingPolicy: {
	defaultOfflineFakesOnly: true
	networkInDefaultSuite: false
	quotaInDefaultSuite: false
}
