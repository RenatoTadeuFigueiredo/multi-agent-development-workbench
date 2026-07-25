// Feature 016 — Grok terminal WorkbenchBackend MVP.
package workbench

#WorkbenchBackendLaunch: {
	args: ["agent", "stdio"]
	executable: string
	workspace:  string
}

#Feature016: {
	crate: "workbench-terminal-backend"
	compatibility_pin: string
}
