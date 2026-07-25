// DDD role: ValueObject

package schemas

// #DurableCostLedgerAndOpenrouterLiveHttps models Feature 014 / issue #31.
// DDD role: ValueObject
#DurableCostLedgerAndOpenrouterLiveHttps: {
	ledger: #DurableCostLedgerPolicy
	openrouter: #OpenRouterLiveHttpsPolicy
}

// DDD role: ValueObject
#DurableCostLedgerPolicy: {
	unit: "usd-micros"
	scope: "per-session"
	persistSecrets: false
	persistRawBodies: false
	restoreOnDaemonStart: true
}

// DDD role: ValueObject
#OpenRouterLiveHttpsPolicy: {
	defaultTransport: "offline-fake"
	liveStack: "rustls-tokio"
	liveEnablement: "explicit-constructor"
	liveSmoke: "ignored-opt-in"
}
