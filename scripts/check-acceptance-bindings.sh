#!/usr/bin/env sh
# Inventory: every committed Gherkin feature under doc/arch/specs/features must
# have a repository-owned workbench-testkit acceptance harness.
#
# Speckit verify (ADR-0020) executes against Speckit's in-binary step registry
# only. It cannot load external Rust step bindings, so unbound Gherkin steps
# from workbench-testkit are expected and advisory. This script is the
# authoritative map from .feature files to Rust harnesses; make test-acceptance
# remains the executable offline gate.

set -eu

repository_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
	printf '%s\n' "error: run inside the Workbench Git repository" >&2
	exit 1
}
cd "$repository_root"

features_dir="doc/arch/specs/features"
tests_dir="crates/workbench-testkit/tests"

if [ ! -d "$features_dir" ]; then
	printf '%s\n' "error: missing $features_dir" >&2
	exit 1
fi
if [ ! -d "$tests_dir" ]; then
	printf '%s\n' "error: missing $tests_dir" >&2
	exit 1
fi

# Fixed map: Speckit feature slug -> workbench-testkit integration test module.
# Keep ordered by feature number; extend when a new delivered feature ships.
missing=0
checked=0

check_pair() {
	number=$1
	slug=$2
	feature_path="$features_dir/${slug}.feature"
	test_path="$tests_dir/feature_${number}.rs"

	checked=$((checked + 1))

	if [ ! -f "$feature_path" ]; then
		printf 'MISSING feature: %s\n' "$feature_path" >&2
		missing=$((missing + 1))
		return
	fi
	if [ ! -f "$test_path" ]; then
		printf 'MISSING harness: %s (for %s)\n' "$test_path" "$feature_path" >&2
		missing=$((missing + 1))
		return
	fi

	# Harness must include the committed feature file (fingerprint source).
	if ! grep -q "$slug" "$test_path"; then
		printf 'UNLINKED harness: %s does not reference slug %s\n' "$test_path" "$slug" >&2
		missing=$((missing + 1))
		return
	fi

	printf 'OK  %s -> %s\n' "$feature_path" "$test_path"
}

check_pair 001 build-the-workbench-orchestration-kernel-foundation-as-a
check_pair 002 create-a-thin-replaceable-vs-code-extension-bridge-to-the
check_pair 003 add-a-versioned-session-list-command-to-the-local-workbench
check_pair 004 add-a-supervised-acp-adapter-for-an-externally-installed
check_pair 005 add-a-supervised-claude-code-subscription-adapter-that-pins
check_pair 006 add-a-supervised-codex-subscription-adapter-that-pins-an
check_pair 007 central-mcp-lifecycle-and-tool-permissions
check_pair 008 execute-configurable-multi-agent-workflows-that-resolve
check_pair 009 add-real-time-vs-code-workflow-controls-that-show-routing
check_pair 010 add-an-openrouter-api-provider-with-cost-controls
check_pair 011 workbench-acp-server-and-terminal-client
check_pair 012 attach-acp-agent-stdio-to-running-daemon
check_pair 013 compose-tls-https-client-for-non-loopback-mcp-endpoints-so
check_pair 014 durable-cost-ledger-and-openrouter-live-https

# Ghost / renumbered paths must not leave orphan feature files unmapped.
orphan=0
for feature_path in "$features_dir"/*.feature; do
	[ -f "$feature_path" ] || continue
	base=$(basename "$feature_path" .feature)
	case "$base" in
	build-the-workbench-orchestration-kernel-foundation-as-a | \
	create-a-thin-replaceable-vs-code-extension-bridge-to-the | \
	add-a-versioned-session-list-command-to-the-local-workbench | \
	add-a-supervised-acp-adapter-for-an-externally-installed | \
	add-a-supervised-claude-code-subscription-adapter-that-pins | \
	add-a-supervised-codex-subscription-adapter-that-pins-an | \
	central-mcp-lifecycle-and-tool-permissions | \
	execute-configurable-multi-agent-workflows-that-resolve | \
	add-real-time-vs-code-workflow-controls-that-show-routing | \
	add-an-openrouter-api-provider-with-cost-controls | \
	workbench-acp-server-and-terminal-client | \
	attach-acp-agent-stdio-to-running-daemon | \
	compose-tls-https-client-for-non-loopback-mcp-endpoints-so | \
	durable-cost-ledger-and-openrouter-live-https) ;;
	*)
		printf 'ORPHAN feature (no harness map entry): %s\n' "$feature_path" >&2
		orphan=$((orphan + 1))
		;;
	esac
done

printf '\nChecked %s feature/harness pairs\n' "$checked"

if [ "$missing" -ne 0 ] || [ "$orphan" -ne 0 ]; then
	printf 'error: acceptance binding inventory failed (missing=%s orphan=%s)\n' \
		"$missing" "$orphan" >&2
	exit 1
fi

printf '%s\n' "Acceptance binding inventory green (Features 001–014)."
printf '%s\n' "Note: speckit verify unbound steps remain advisory — Speckit's executable registry is binary-local (ADR-0020) and does not load external Rust harnesses. Authoritative gate: make test-acceptance."
