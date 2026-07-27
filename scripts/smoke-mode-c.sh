#!/usr/bin/env sh
# Offline Mode C precondition checks (no live models, no daemon required).
#
# Validates env and path shape for Grok TUI → workbench agent stdio → daemon.
# Does not start processes, call providers, or require network.
#
# Usage:
#   ./scripts/smoke-mode-c.sh
#   WORKBENCH_EXECUTABLE=/abs/path/to/workbench ./scripts/smoke-mode-c.sh
#   GROK_BUILD_ROOT=/abs/path/to/grok-build ./scripts/smoke-mode-c.sh
#
# Exit 0 when all hard checks pass; non-zero if a required precondition fails.
# Soft warnings (daemon socket absent, binary not built) print but do not fail
# unless SMOKE_MODE_C_STRICT=1.

set -eu

section() {
	printf '\n## %s\n' "$1"
}

ok() {
	printf 'OK  %s\n' "$1"
}

warn() {
	printf 'WARN %s\n' "$1"
	WARNINGS=$((WARNINGS + 1))
}

fail() {
	printf 'FAIL %s\n' "$1" >&2
	FAILURES=$((FAILURES + 1))
}

is_truthy() {
	case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
	1 | true | yes | on) return 0 ;;
	*) return 1 ;;
	esac
}

is_absolute() {
	case "${1:-}" in
	/*) return 0 ;;
	*) return 1 ;;
	esac
}

has_parent_traversal() {
	case "/${1:-}/" in
	*/../*) return 0 ;;
	*) return 1 ;;
	esac
}

FAILURES=0
WARNINGS=0
STRICT=0
if is_truthy "${SMOKE_MODE_C_STRICT:-0}"; then
	STRICT=1
fi

REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$REPO_ROOT"

printf '%s\n' "# Mode C offline smoke"
printf 'Repository: %s\n' "$REPO_ROOT"

section "Workbench CLI path"

WB_EXE="${WORKBENCH_EXECUTABLE:-}"
if [ -z "$WB_EXE" ]; then
	# Prefer a workspace-built binary when present (debug then release).
	for candidate in \
		"$REPO_ROOT/target/debug/workbench" \
		"$REPO_ROOT/target/release/workbench"
	do
		if [ -x "$candidate" ]; then
			WB_EXE=$candidate
			break
		fi
	done
fi

if [ -z "$WB_EXE" ]; then
	fail "WORKBENCH_EXECUTABLE unset and no target/{debug,release}/workbench binary found"
elif ! is_absolute "$WB_EXE"; then
	fail "WORKBENCH_EXECUTABLE must be absolute (got: $WB_EXE)"
elif has_parent_traversal "$WB_EXE"; then
	fail "WORKBENCH_EXECUTABLE must not contain parent traversal (got: $WB_EXE)"
elif [ ! -e "$WB_EXE" ]; then
	fail "WORKBENCH_EXECUTABLE does not exist: $WB_EXE"
elif [ ! -x "$WB_EXE" ]; then
	fail "WORKBENCH_EXECUTABLE is not executable: $WB_EXE"
else
	ok "workbench executable: $WB_EXE"
fi

section "Backend selection (Mode C)"

SELECTED=0
if is_truthy "${WORKBENCH_TERMINAL_BACKEND:-}"; then
	ok "WORKBENCH_TERMINAL_BACKEND=${WORKBENCH_TERMINAL_BACKEND}"
	SELECTED=1
fi
case "${GROK_AGENT_BACKEND:-}" in
workbench | WORKBENCH)
	ok "GROK_AGENT_BACKEND=${GROK_AGENT_BACKEND}"
	SELECTED=1
	;;
esac

if [ "$SELECTED" -eq 0 ]; then
	warn "neither WORKBENCH_TERMINAL_BACKEND=1 nor GROK_AGENT_BACKEND=workbench is set (Mode C not selected yet)"
fi

section "GROK_HOME isolation (fork profile)"

GROK_HOME_VAL="${GROK_HOME:-}"
if [ -z "$GROK_HOME_VAL" ]; then
	warn "GROK_HOME unset (fork would use default ~/.grok; recommend ~/.grokdev for isolation)"
else
	if ! is_absolute "$GROK_HOME_VAL"; then
		fail "GROK_HOME must be absolute (got: $GROK_HOME_VAL)"
	elif [ ! -d "$GROK_HOME_VAL" ]; then
		warn "GROK_HOME does not exist yet: $GROK_HOME_VAL (mkdir -p && chmod 0700 before launch)"
	else
		ok "GROK_HOME=$GROK_HOME_VAL"
	fi
fi

if [ -n "${GROK_LEADER_SOCKET:-}" ]; then
	if is_absolute "$GROK_LEADER_SOCKET"; then
		ok "GROK_LEADER_SOCKET=$GROK_LEADER_SOCKET"
	else
		fail "GROK_LEADER_SOCKET must be absolute (got: $GROK_LEADER_SOCKET)"
	fi
else
	warn "GROK_LEADER_SOCKET unset (isolation profile usually sets \$GROK_HOME/leader.sock)"
fi

section "Daemon socket (soft)"

# Workbench daemon socket is workspace-local; probe common layout only.
SOCKET_CANDIDATES=""
if [ -n "${WORKBENCH_SOCKET:-}" ]; then
	SOCKET_CANDIDATES="$WORKBENCH_SOCKET"
fi
# Config lock / runtime dir conventions vary; report presence when discoverable.
if [ -d "$REPO_ROOT/.workbench" ]; then
	for s in "$REPO_ROOT/.workbench"/*.sock "$REPO_ROOT/.workbench"/daemon.sock; do
		if [ -e "$s" ]; then
			SOCKET_CANDIDATES="$SOCKET_CANDIDATES $s"
		fi
	done
fi

FOUND_SOCKET=0
for s in $SOCKET_CANDIDATES; do
	if [ -S "$s" ] || [ -e "$s" ]; then
		ok "daemon socket present: $s"
		FOUND_SOCKET=1
	fi
done
if [ "$FOUND_SOCKET" -eq 0 ]; then
	warn "no daemon socket found (offline OK; live Mode C needs workbench daemon in the workspace)"
fi

section "Compatibility pin"

PIN_FILE="$REPO_ROOT/crates/workbench-terminal-backend/src/lib.rs"
if [ -f "$PIN_FILE" ]; then
	PIN=$(sed -n 's/.*GROK_BUILD_FORK_COMPATIBILITY_PIN: &str = "\([0-9a-f]\{40\}\)".*/\1/p' "$PIN_FILE" | head -n 1)
	if [ -n "$PIN" ]; then
		ok "GROK_BUILD_FORK_COMPATIBILITY_PIN=$PIN"
	else
		warn "could not parse GROK_BUILD_FORK_COMPATIBILITY_PIN from $PIN_FILE"
	fi
else
	fail "missing pin source: $PIN_FILE"
fi

section "Grok Build fork (optional)"

GROK_BUILD_ROOT="${GROK_BUILD_ROOT:-}"
if [ -z "$GROK_BUILD_ROOT" ] && [ -d "${HOME}/Projects/grok-build" ]; then
	GROK_BUILD_ROOT="${HOME}/Projects/grok-build"
fi

if [ -n "$GROK_BUILD_ROOT" ]; then
	if [ ! -d "$GROK_BUILD_ROOT" ]; then
		warn "GROK_BUILD_ROOT not a directory: $GROK_BUILD_ROOT"
	else
		ok "GROK_BUILD_ROOT=$GROK_BUILD_ROOT"
		WB_MOD="$GROK_BUILD_ROOT/crates/codegen/xai-grok-pager/src/acp/workbench_backend.rs"
		if [ -f "$WB_MOD" ]; then
			ok "fork WorkbenchBackend module present"
		else
			warn "fork missing workbench_backend.rs (need pin SHA / feature/fcustom-mode-c tip; prior monorepo-sync Mode C was 85989c9)"
		fi
		if [ -n "${PIN:-}" ] && command -v git >/dev/null 2>&1; then
			if git -C "$GROK_BUILD_ROOT" cat-file -e "${PIN}^{commit}" 2>/dev/null; then
				ok "fork contains pin commit $PIN"
			else
				warn "fork does not contain pin commit $PIN (fetch/rebase may be needed)"
			fi
			HEAD=$(git -C "$GROK_BUILD_ROOT" rev-parse HEAD 2>/dev/null || true)
			if [ -n "$HEAD" ]; then
				ok "fork HEAD=$HEAD"
			fi
		fi
	fi
else
	warn "GROK_BUILD_ROOT unset and ~/Projects/grok-build absent (skip fork checks)"
fi

section "Summary"

printf 'failures=%s warnings=%s strict=%s\n' "$FAILURES" "$WARNINGS" "$STRICT"

if [ "$FAILURES" -gt 0 ]; then
	exit 1
fi
if [ "$STRICT" -eq 1 ] && [ "$WARNINGS" -gt 0 ]; then
	exit 1
fi
exit 0
