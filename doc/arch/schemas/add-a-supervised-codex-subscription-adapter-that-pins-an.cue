// DDD role: ValueObject

package schemas

// #AddASupervisedCodexSubscriptionAdapterThatPinsAn models the locked Codex
// exec JSONL adapter.
// DDD role: ValueObject
#AddASupervisedCodexSubscriptionAdapterThatPinsAn: {
	adapter:      "codex"
	providerType: "subscription-cli"
	protocol:     "codex-exec-jsonl/1"
	identity: {
		executable:       string & =~"^/"
		executableSha256: string & =~"^[0-9a-f]{64}$"
		version:          string & !=""
	}
	launch: {
		subcommand:          "exec"
		jsonEvents:          true
		ephemeral:           true
		sandbox:             "read-only"
		approvalBypass:      false
		workspaceWrite:      false
		dangerFullAccess:    false
		providerSessionPersist: false
	}
	limits: {
		maxFrameBytes:                      8388608
		cancellationProviderMs:             4500
		cancellationFinalizationReserveMs:  500
	}
	authentication: {
		owner:                       "codex-cli"
		requiredMode:                "chatgpt-subscription"
		workbenchHandlesCredentials: false
		credentialPathsForbidden: [
			"auth.json",
			"CODEX_HOME credential store",
		]
	}
}
