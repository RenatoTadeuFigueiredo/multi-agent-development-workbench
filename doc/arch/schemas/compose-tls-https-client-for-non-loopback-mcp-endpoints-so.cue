// DDD role: ValueObject

package schemas

// #ComposeTlsHttpsClientForNonLoopbackMcpEndpointsSo models TLS policy for
// the central MCP HTTP client (Feature 013 / issue #30).
// DDD role: ValueObject
#ComposeTlsHttpsClientForNonLoopbackMcpEndpointsSo: {
	tls: #McpTlsTransportPolicy
	testing: #McpTlsTestingPolicy
}

// DDD role: ValueObject
#McpTlsTransportPolicy: {
	tlsStack: "rustls-tokio"
	nonLoopbackRequiresHttps: true
	httpsUsesTls: true
	loopbackHttpCleartextAllowed: true
	rejectUnpinnedRedirect: true
	maxEncodedResponseBytes: 8388608
	productionTrust: "rustls-native-certs"
}

// DDD role: ValueObject
#McpTlsTestingPolicy: {
	defaultTests: "offline-fakes-or-local-tls-fixture"
	livePublicHttps: "ignored-opt-in"
	forbidReqwest: true
	forbidHyper: true
}
