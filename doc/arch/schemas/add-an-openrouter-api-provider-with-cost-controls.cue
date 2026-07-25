// DDD role: ValueObject

package schemas

// #AddAnOpenrouterApiProviderWithCostControls models the OpenRouter API adapter.
// DDD role: ValueObject
#AddAnOpenrouterApiProviderWithCostControls: {
	protocol: "openrouter-chat-completions/1"
	providerType: "api"
	defaultBaseURL: "https://openrouter.ai/api/v1"
	credential: #OpenRouterCredentialPolicy
	budget: #OpenRouterBudgetPolicy
	runtime: #OpenRouterRuntimePolicy
	testing: #OpenRouterTestingPolicy
}

// DDD role: ValueObject
#OpenRouterCredentialPolicy: {
	credentialRefRequired: true
	plaintextSecretsInConfig: false
	acceptedPrefixPlatform: true
	acceptedPrefixKeychain: true
	acceptedPrefixSecretService: true
	missingCredential: "fail-closed-pre-dispatch"
}

// DDD role: ValueObject
#OpenRouterBudgetPolicy: {
	requiredWhenApiProviderPresent: true
	maxSessionUsdMicrosRequired: true
	maxAttemptUsdMicrosOptional: true
	enforceBeforeDispatch: true
	overBudget: "fail-closed-pre-dispatch"
}

// DDD role: ValueObject
#OpenRouterRuntimePolicy: {
	streaming: true
	toolCallingClaimed: false
	maxEncodedBodyBytes: 8388608
	materialCost: true
	effectClass: "paid-inference"
	privacyRequired: true
}

// DDD role: ValueObject
#OpenRouterTestingPolicy: {
	defaultOfflineFakesOnly: true
	networkInDefaultSuite: false
	quotaInDefaultSuite: false
	liveTestsIgnoredByDefault: true
}
