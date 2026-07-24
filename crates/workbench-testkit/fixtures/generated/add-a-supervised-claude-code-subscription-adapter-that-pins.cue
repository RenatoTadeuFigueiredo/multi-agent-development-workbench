// DDD role: ValueObject

package schemas

// #AddASupervisedClaudeCodeSubscriptionAdapterThatPins models the locked Claude Code stream adapter.
// DDD role: ValueObject
#AddASupervisedClaudeCodeSubscriptionAdapterThatPins: {
    adapter: "claude-code"
    providerType: "subscription-cli"
    protocol: "claude-code-stream-json/1"
    identity: {
        executable: string & =~"^/"
        executableSha256: string & =~"^[0-9a-f]{64}$"
        version: string & !=""
    }
    launch: {
        inputFormat: "stream-json"
        outputFormat: "stream-json"
        verbose: true
        partialMessages: true
        permissionMode: "dontAsk"
        noSessionPersistence: true
        strictMcp: true
        chrome: false
        slashCommands: false
        tools: ["Read", "Glob", "Grep"]
    }
    limits: {
        maxFrameBytes: 8388608
        cancellationProviderMs: 4500
        cancellationFinalizationReserveMs: 500
    }
    authentication: {
        owner: "claude-code"
        requiredMode: "subscription"
        workbenchHandlesCredentials: false
    }
}
