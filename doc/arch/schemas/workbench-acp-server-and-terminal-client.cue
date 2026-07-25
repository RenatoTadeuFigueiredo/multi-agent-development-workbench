// DDD role: ValueObject

package schemas

#WorkbenchAcpServerAndTerminalClient: {
	protocolVersion: 1
	stdioAgentCommand: "workbench agent stdio"
	reusesDaemonProtocol: true
	embedsGrok: false
	frameCeilingBytes: 8388608
	testing: {
		defaultOfflineOnly: true
		networkInDefaultSuite: false
	}
	deferred: {
		grokTerminalFork: true
		editorPackaging: true
	}
}
