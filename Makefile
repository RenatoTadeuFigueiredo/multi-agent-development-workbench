# Makefile for multi-agent-development-workbench
#
# Self-documenting: run `make` (or `make help`) to list the available targets.
# Build/Test targets report the current specification-only state. The active
# Speckit implementation phase will replace them with the pinned Cargo commands.

.DEFAULT_GOAL := help

.PHONY: help build test lint check validate verify analyze spec-status

help: ## Show this help
	@awk 'BEGIN {FS = ":.*##"; printf "Usage: make <target>\n\nTargets:\n"} /^[a-zA-Z0-9_-]+:.*##/ {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ---------------------------------------------------------------------------
# Build / Test — no product workspace exists before the implementation phase.
# ---------------------------------------------------------------------------

build: ## Report product build availability
	@echo "No product build exists during the specification phase"

test: ## Report product test availability
	@echo "No product tests exist before the implementation phase"

# ---------------------------------------------------------------------------
# Spec — speckit spec-driven workflow gates (ready to use).
# ---------------------------------------------------------------------------

validate: ## Validate the doc/arch spec corpus
	speckit validate

lint: validate ## Run the documentation lint gate

verify: ## Verify the executable specs against the implementation
	speckit verify

analyze: ## Analyze the spec corpus for gaps and drift
	speckit analyze

spec-status: ## Show the active feature and current workflow phase
	speckit status

check: validate verify test ## Run the non-mutating CI gate
