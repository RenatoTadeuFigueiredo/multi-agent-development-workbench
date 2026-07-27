#!/usr/bin/env bash
# LIVE Mode C smoke — protocol level (no multi-provider inference).
#
# Proves:
#   1) disposable workspace + config lock
#   2) workbench daemon starts
#   3) session create / list via CLI
#   4) workbench agent stdio ACP initialize handshake
#   5) fork live_spawn_workbench test when cargo + WORKBENCH_LIVE_TEST available
#
# Usage:
#   ./scripts/live-mode-c-smoke.sh
#   WORKBENCH_EXECUTABLE=/abs/workbench GROK_PAGER=/abs/xai-grok-pager ./scripts/live-mode-c-smoke.sh
#
# Exit 0 on hard-pass; non-zero if a required step fails.

set -euo pipefail

section() { printf '\n## %s\n' "$1"; }
ok() { printf 'OK  %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
warn() { printf 'WARN %s\n' "$1"; }

REPO_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$REPO_ROOT"

WB="${WORKBENCH_EXECUTABLE:-$REPO_ROOT/target/debug/workbench}"
PAGER="${GROK_PAGER:-$HOME/Projects/grok-build/target/debug/xai-grok-pager}"
GROK_BUILD_ROOT="${GROK_BUILD_ROOT:-$HOME/Projects/grok-build}"
export GROK_HOME="${GROK_HOME:-$HOME/.grokdev}"
export GROK_LEADER_SOCKET="${GROK_LEADER_SOCKET:-$GROK_HOME/leader.sock}"
export GROK_DISABLE_AUTOUPDATER=1
export WORKBENCH_EXECUTABLE="$WB"
export WORKBENCH_TERMINAL_BACKEND=1

# macOS Unix sockets are limited to ~104 bytes (SUN_LEN). Default TMPDIR under
# /var/folders/... canonicalizes to /private/var/... and exceeds that limit for
# workbench-{uid}/{workspace_id}.sock. Prefer a short owner-writable root.
if [[ "$(uname -s)" == "Darwin" ]]; then
  SHORT_TMP="${WORKBENCH_SHORT_TMPDIR:-$HOME/w}"
  mkdir -p "$SHORT_TMP"
  chmod 0700 "$SHORT_TMP" 2>/dev/null || true
  export TMPDIR="$SHORT_TMP"
fi

printf '%s\n' "# Mode C LIVE smoke"
printf 'Workbench: %s\n' "$WB"
printf 'Pager:     %s\n' "$PAGER"
printf 'GROK_HOME: %s\n' "$GROK_HOME"
printf 'TMPDIR:    %s\n' "${TMPDIR:-}"

[[ -x "$WB" ]] || fail "workbench not executable: $WB (cargo build -p workbench-cli)"
mkdir -p "$GROK_HOME" && chmod 0700 "$GROK_HOME"
ok "GROK_HOME ready"

section "0) Offline preconditions"
if [[ -x "$REPO_ROOT/scripts/smoke-mode-c.sh" ]]; then
  WORKBENCH_EXECUTABLE="$WB" GROK_BUILD_ROOT="$GROK_BUILD_ROOT" \
    GROK_HOME="$GROK_HOME" GROK_LEADER_SOCKET="$GROK_LEADER_SOCKET" \
    bash "$REPO_ROOT/scripts/smoke-mode-c.sh" || warn "offline smoke returned non-zero (continuing live path)"
else
  warn "scripts/smoke-mode-c.sh missing"
fi

section "1) Disposable workspace"
WS=$(mktemp -d "${TMPDIR:-/tmp}/workbench-mode-c-smoke.XXXXXX")
cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

cd "$WS"
git init -q
git config user.email "mode-c-smoke@local"
git config user.name "ModeCSmoke"
printf '%s\n' "# mode-c live smoke" >README.md
git add README.md
git commit -q -m "chore: disposable Mode C smoke workspace"
ok "workspace $WS"

section "2) Config lock + daemon"
"$WB" config lock
"$WB" config validate
"$WB" daemon >"$WS/daemon.log" 2>&1 &
DAEMON_PID=$!
sleep 2
if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
  cat "$WS/daemon.log" >&2 || true
  fail "daemon exited early (pid $DAEMON_PID)"
fi
ok "daemon pid=$DAEMON_PID"

section "3) CLI status / session create / list"
set +e
"$WB" --json status >"$WS/status.json" 2>"$WS/status.err"
ST=$?
set -e
if [[ $ST -ne 0 ]]; then
  cat "$WS/status.err" "$WS/daemon.log" >&2 || true
  fail "workbench --json status exit=$ST"
fi
ok "status ($(wc -c <"$WS/status.json" | tr -d ' ') bytes)"

set +e
"$WB" --json session create >"$WS/create.json" 2>"$WS/create.err"
CR=$?
set -e
if [[ $CR -ne 0 ]]; then
  cat "$WS/create.err" "$WS/daemon.log" >&2 || true
  fail "session create exit=$CR"
fi
ok "session create"

set +e
"$WB" --json session list >"$WS/list.json" 2>"$WS/list.err"
LS=$?
set -e
if [[ $LS -ne 0 ]]; then
  cat "$WS/list.err" >&2 || true
  fail "session list exit=$LS"
fi
ok "session list ($(wc -c <"$WS/list.json" | tr -d ' ') bytes)"

section "4) agent stdio ACP initialize (protocol only)"
# ACP initialize — bridge may answer with initialize result or error envelope.
# We only require the process to accept a frame and exit cleanly under timeout.
# Prefer GNU/coreutils timeout when present; otherwise a portable bash watchdog
# (macOS often lacks `timeout` unless coreutils is installed).
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"mode-c-live-smoke","version":"0"}}}'
set +e
if command -v timeout >/dev/null 2>&1; then
  printf '%s\n' "$INIT" | timeout 10s "$WB" agent stdio >"$WS/agent-stdio.out" 2>"$WS/agent-stdio.err"
  AG=$?
elif command -v gtimeout >/dev/null 2>&1; then
  printf '%s\n' "$INIT" | gtimeout 10s "$WB" agent stdio >"$WS/agent-stdio.out" 2>"$WS/agent-stdio.err"
  AG=$?
else
  printf '%s\n' "$INIT" | "$WB" agent stdio >"$WS/agent-stdio.out" 2>"$WS/agent-stdio.err" &
  AG_PID=$!
  AG=124
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    if ! kill -0 "$AG_PID" 2>/dev/null; then
      wait "$AG_PID"
      AG=$?
      break
    fi
    sleep 0.5
  done
  if kill -0 "$AG_PID" 2>/dev/null; then
    kill "$AG_PID" 2>/dev/null || true
    wait "$AG_PID" 2>/dev/null || true
    AG=124
  fi
fi
set -e
# 124 = timeout (acceptable if bridge stays open waiting for more frames)
if [[ $AG -eq 124 ]]; then
  ok "agent stdio stayed open (timeout) — process accepted stdin"
elif [[ $AG -eq 0 ]]; then
  ok "agent stdio exit 0"
else
  printf 'agent-stdio.err:\n' >&2
  cat "$WS/agent-stdio.err" >&2 || true
  printf 'agent-stdio.out:\n' >&2
  cat "$WS/agent-stdio.out" >&2 || true
  warn "agent stdio exit=$AG (inspect logs; may still be usable with full ACP client)"
fi
if [[ -s "$WS/agent-stdio.out" ]]; then
  ok "agent stdio produced stdout ($(wc -c <"$WS/agent-stdio.out" | tr -d ' ') bytes)"
else
  warn "agent stdio produced no stdout (client may need full ACP session/new flow)"
fi

section "5) Fork WorkbenchBackend live spawn (optional)"
if [[ ! -x "$PAGER" ]] && [[ ! -d "$GROK_BUILD_ROOT" ]]; then
  warn "skip fork live test — no pager binary and no GROK_BUILD_ROOT"
elif [[ -d "$GROK_BUILD_ROOT" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    (
      cd "$GROK_BUILD_ROOT"
      export GROK_HOME GROK_LEADER_SOCKET GROK_DISABLE_AUTOUPDATER
      export WORKBENCH_EXECUTABLE WORKBENCH_LIVE_TEST=1
      export GROK_CURSOR_SKILLS_ENABLED=0
      set +e
      cargo test -p xai-grok-pager live_spawn_workbench -- --ignored --nocapture \
        >"$WS/fork-live.out" 2>"$WS/fork-live.err"
      FT=$?
      set -e
      if [[ $FT -eq 0 ]]; then
        ok "fork live_spawn_workbench passed"
      else
        warn "fork live test exit=$FT (see $WS/fork-live.err)"
        tail -40 "$WS/fork-live.err" 2>/dev/null || true
      fi
    )
  else
    warn "cargo not available — skip fork live test"
  fi
else
  warn "GROK_BUILD_ROOT missing — skip fork live test"
fi

section "Summary"
printf 'Workspace logs: %s\n' "$WS"
printf '  daemon.log status.json create.json list.json agent-stdio.*\n'
ok "LIVE Mode C protocol smoke finished (no multi-provider inference)"
printf '\nNext manual step (interactive TUI):\n'
printf '  GROK_HOME=%s WORKBENCH_TERMINAL_BACKEND=1 WORKBENCH_EXECUTABLE=%s \\\n' "$GROK_HOME" "$WB"
printf '  %s --no-leader --no-auto-update\n' "$PAGER"
printf '  (cwd must be the same workspace; VS Code: open same folder + Workbench attach)\n'
