// DDD role: ValueObject

package schemas

// #AddASupervisedAcpAdapterForAnExternallyInstalled captures the fixed,
// capability-negotiated Grok Build ACP provider profile.
// DDD role: ValueObject
#AddASupervisedAcpAdapterForAnExternallyInstalled: {
    launch: #GrokAcpLaunch
    transport: #GrokAcpTransport
    compatibility: #GrokAcpCompatibility
    authorization: #GrokAcpAuthorization
    cancellation: #GrokAcpCancellation
    lifecycle: #GrokAcpLifecycle
}

#GrokAcpLaunch: {
    executable: string & !=""
    executable_sha256: string & =~"^[a-f0-9]{64}$"
    command: #GrokAcpCommand
    environment: {
        GROK_DISABLE_AUTOUPDATER: "1"
    }
    workspace_id: string & =~"^[a-f0-9]{32}$"
    shell: false
}

#GrokAcpCommand: {
    values: ["agent", "--no-leader", "stdio"]
}

#GrokAcpTransport: {
    jsonrpc: "2.0"
    framing: "ndjson"
    encoding: "utf-8"
    full_duplex: true
    max_frame_bytes: 8388608
}

#GrokAcpCompatibility: {
    protocol_version: 1
    required_methods: #GrokAcpMethods
    additive_unknown_fields: "ignore"
    executable_pin: "protocol-version-sha256"
}

#GrokAcpMethods: {
    values: [
        "initialize",
        "authenticate",
        "session/new",
        "session/load",
        "session/prompt",
        "session/cancel",
        "session/update",
    ]
}

#GrokAcpAuthorization: {
    authentication_owner: "grok-build"
    reverse_permission: "deny"
}

#GrokAcpCancellation: {
    deadline_ms: 5000
    confirmed_by: "prompt-stop-reason-cancelled"
}

#GrokAcpLifecycle: {
    automatic_update: false
    unsupported: #GrokAcpUnsupported
}

#GrokAcpUnsupported: {
    values: ["pause", "x.ai-interject"]
}
