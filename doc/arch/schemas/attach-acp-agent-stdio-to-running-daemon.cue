// DDD role: ValueObject

package schemas

// #AttachAcpAgentStdioToRunningDaemon models production ACP attach policy
// (Feature 012 / issue #29).
// DDD role: ValueObject
#AttachAcpAgentStdioToRunningDaemon: {
	attach: #AcpDaemonAttachPolicy
	testing: #AcpDaemonAttachTestingPolicy
}

// DDD role: ValueObject
#AcpDaemonAttachPolicy: {
	productionBackend: "daemon-socket"
	transport: "unix-ndjson-workbench-v1"
	discoverRuntimePaths: true
	missingDaemon: "fail-closed"
	inProcessOfflineFakeRetained: true
}

// DDD role: ValueObject
#AcpDaemonAttachTestingPolicy: {
	defaultTests: "offline-local-daemon-harness"
	liveProviders: "forbidden-in-default-suite"
}
