# Makefile for multi-agent-development-workbench
#
# Self-documenting: run `make` (or `make help`) to list the available targets.
# The default gate is deterministic and offline. Platform credential-store
# coverage is explicit because it requires an unlocked operating-system store.

.DEFAULT_GOAL := help

.PHONY: supply-chain-policy secret-scan advisory-check-ci \
	license-source-check-ci sbom supply-chain-ci
.PHONY: help context test-context build fmt lint test contract-test test-platform \
	test-acceptance test-acceptance-bindings test-acp test-acp-attach test-claude \
	test-codex test-mcp-tls test-openrouter-durable test-provider-writes \
	test-terminal-backend test-slo check spec-gate validate verify analyze \
	spec-status smoke-mode-c live-mode-c-smoke

CARGO_OFFLINE := CARGO_NET_OFFLINE=true cargo

help: ## Show this help
	@awk 'BEGIN {FS = ":.*##"; printf "Usage: make <target>\n\nTargets:\n"} /^[a-zA-Z0-9_-]+:.*##/ {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

context: ## Reconstruct the current project state for a fresh session
	@./scripts/project-context.sh

test-context: ## Validate the project-context entry point without network access
	sh -n ./scripts/project-context.sh
	@WORKBENCH_CONTEXT_OFFLINE=1 ./scripts/project-context.sh >/dev/null 2>&1

smoke-mode-c: ## Offline Mode C precondition smoke (paths, pin, optional fork)
	@./scripts/smoke-mode-c.sh

live-mode-c-smoke: ## LIVE Mode C protocol smoke (daemon, session, ACP stdio; no inference)
	@./scripts/live-mode-c-smoke.sh

build: ## Build every Rust workspace crate
	$(CARGO_OFFLINE) build --workspace --locked

fmt: ## Verify Rust formatting
	$(CARGO_OFFLINE) fmt --all -- --check

lint: fmt ## Run Clippy with warnings denied
	$(CARGO_OFFLINE) clippy --workspace --all-targets --all-features --locked -- -D warnings

test: ## Run the default offline Rust test suite
	$(CARGO_OFFLINE) test --workspace --locked

contract-test: ## Reject drift from the committed architecture contracts
	./scripts/check-contract-drift.sh
	$(CARGO_OFFLINE) test -p workbench-testkit --test contract_fixtures --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test client_contract --locked

test-platform: ## Exercise the real OS key store (requires an unlocked credential store)
	$(CARGO_OFFLINE) test -p workbench-storage --test key_store_contract --locked \
		platform_key_store_obeys_the_common_contract -- \
		--exact --ignored --test-threads=1

test-acp: ## Run the deterministic offline ACP subprocess profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_004 --locked

test-claude: ## Run the deterministic offline Claude subprocess profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_005 --locked

test-codex: ## Run the deterministic offline Codex subprocess profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_006 --locked

test-mcp: ## Run the deterministic offline MCP gateway profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_007 --locked

test-workflow: ## Run the deterministic offline workflow executor profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_008 --locked

test-vscode-controls: ## Run the deterministic offline VS Code workflow control profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_009 --locked

test-openrouter: ## Run the deterministic offline OpenRouter profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_010 --locked
	$(CARGO_OFFLINE) test -p workbench-openrouter --locked

test-openrouter-durable: ## Run the offline durable cost ledger profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_014 --locked

test-provider-writes: ## Run the offline provider-native write policy profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_015 --locked

test-acp-server: ## Run the deterministic offline Workbench ACP server profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_011 --locked

test-acp-attach: ## Run the offline ACP agent attach-to-daemon profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_012 --locked

test-mcp-tls: ## Run the offline non-loopback HTTPS MCP TLS profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_013 --locked
	$(CARGO_OFFLINE) test -p workbench-mcp --locked

test-terminal-backend: ## Run the offline WorkbenchBackend terminal integration profile
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_016 --locked
	$(CARGO_OFFLINE) test -p workbench-terminal-backend --locked

test-acceptance-bindings: ## Inventory Gherkin features vs workbench-testkit harnesses
	./scripts/check-acceptance-bindings.sh

test-acceptance: test-acceptance-bindings ## Run all committed feature acceptance harnesses
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_001 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_002 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_003 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_004 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_005 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_006 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_007 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_008 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_009 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_010 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_011 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_012 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_013 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_014 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_015 --locked
	$(CARGO_OFFLINE) test -p workbench-testkit --test feature_016 --locked

test-slo: ## Run serialized feature 001 SLO measurements
	$(CARGO_OFFLINE) test -p workbench-testkit --test slo_001 --locked -- \
		--ignored --nocapture --test-threads=1

supply-chain-policy: ## Validate pinned policy and immutable CI action references
	python3 ./scripts/check-supply-chain-policy.py

secret-scan: ## Scan working-tree text for high-confidence plaintext secrets
	python3 ./scripts/check-secrets.py

advisory-check-ci: ## Fetch RustSec and reject vulnerable, yanked, or abandoned crates
	cargo deny --locked check advisories

license-source-check-ci: ## Enforce the reviewed license and dependency-source policy
	CARGO_NET_OFFLINE=true cargo deny --locked --offline check licenses sources

sbom: ## Generate and structurally validate reproducible CycloneDX 1.5 SBOMs
	./scripts/generate-sbom.sh

supply-chain-ci: supply-chain-policy secret-scan advisory-check-ci license-source-check-ci sbom ## Run network-enabled release supply-chain gates

# ---------------------------------------------------------------------------
# Spec — speckit spec-driven workflow gates (ready to use).
# ---------------------------------------------------------------------------

validate: ## Validate the doc/arch spec corpus
	speckit validate

verify: ## Verify the executable specs against the implementation
	speckit verify

analyze: ## Analyze the spec corpus for gaps and drift
	speckit analyze

spec-gate: ## Run Speckit gates, or the committed corpus fallback when unavailable
	@set -eu; \
	if command -v speckit >/dev/null 2>&1; then \
		speckit analyze; \
		speckit verify; \
		speckit validate; \
	else \
		echo "Speckit binary unavailable; running committed corpus fallback"; \
		./scripts/check-contract-drift.sh; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test contract_fixtures --locked; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test feature_001 --locked; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test feature_002 --locked; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test feature_003 --locked; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test feature_004 --locked; \
		$(CARGO_OFFLINE) test -p workbench-testkit --test feature_005 --locked; \
	fi

spec-status: ## Show the active feature and current workflow phase
	speckit status

check: test-context supply-chain-policy secret-scan lint contract-test test test-acceptance \
	test-slo spec-gate ## Run the complete offline gate
