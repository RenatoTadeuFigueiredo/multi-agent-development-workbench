// DDD role: ValueObject

package schemas

// #CreateAThinReplaceableVsCodeExtensionBridgeToThe models the protocol-facing
// state owned in memory by the VS Code presentation bridge.
// DDD role: ValueObject
#CreateAThinReplaceableVsCodeExtensionBridgeToThe: {
    session_id: string & !=""
    last_sequence: int & >=0
    rendered_events: #RenderedEvents
    endpoint: string & !=""
}

#RenderedEvents: {
    ids: [...string]
}
