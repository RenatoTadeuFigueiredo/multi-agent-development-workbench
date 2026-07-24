#!/usr/bin/env sh

set -eu

section() {
	printf '\n## %s\n' "$1"
}

if ! repository_root=$(git rev-parse --show-toplevel 2>/dev/null); then
	printf '%s\n' "error: run this command inside the Workbench Git repository" >&2
	exit 1
fi

cd "$repository_root"

printf '%s\n' "# Workbench Project Context"
printf 'Repository: %s\n' "$repository_root"

section "Git"

if branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null); then
	printf 'Branch: %s\n' "$branch"
else
	printf 'Branch: detached HEAD\n'
fi

printf 'HEAD: %s\n' "$(git rev-parse HEAD)"

if upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null); then
	set -- $(git rev-list --left-right --count "$upstream...HEAD")
	printf 'Upstream: %s (behind %s, ahead %s; local tracking data)\n' \
		"$upstream" "$1" "$2"
else
	printf '%s\n' "Upstream: none"
fi

if changes=$(git status --porcelain) && [ -n "$changes" ]; then
	printf '%s\n' "Worktree: dirty"
	printf '%s\n' "$changes"
else
	printf '%s\n' "Worktree: clean"
fi

printf '%s\n' "Recent commits:"
git log -5 --format='  %h %s'

section "Durable Handoff"

status_path="docs/project/STATUS.md"
if [ -f "$status_path" ]; then
	printf 'Status document: %s\n' "$status_path"
	next_ready=$(sed -n 's/^- \*\*Next ready:\*\* //p' "$status_path" | sed -n '1p')
	if [ -n "$next_ready" ]; then
		printf 'Next documented work: %s\n' "$next_ready"
	fi
else
	printf 'Status document: missing (%s)\n' "$status_path"
fi

section "Speckit"

if command -v speckit >/dev/null 2>&1; then
	if ! speckit status; then
		printf '%s\n' "warning: speckit is installed but status could not be read"
	fi
else
	printf '%s\n' "unavailable: install Speckit before product feature work"
fi

section "GitHub"

if [ "${WORKBENCH_CONTEXT_OFFLINE:-0}" = "1" ]; then
	printf '%s\n' "skipped: WORKBENCH_CONTEXT_OFFLINE=1"
elif ! command -v gh >/dev/null 2>&1; then
	printf '%s\n' "unavailable: install GitHub CLI to inspect live issues and pull requests"
elif ! gh auth status >/dev/null 2>&1; then
	printf '%s\n' "unavailable: authenticate GitHub CLI with 'gh auth login'"
elif repository=$(gh repo view --json nameWithOwner --jq '.nameWithOwner' 2>/dev/null); then
	printf 'Repository: %s\n' "$repository"
	printf '%s\n' "Open issues:"
	if ! gh issue list --state open --limit 20 --json number,title \
		--template '{{range .}}  #{{.number}} {{.title}}{{"\n"}}{{end}}'; then
		printf '%s\n' "  warning: live issue query failed"
	fi
	printf '%s\n' "Open pull requests:"
	if ! gh pr list --state open --limit 20 --json number,title \
		--template '{{range .}}  #{{.number}} {{.title}}{{"\n"}}{{end}}'; then
		printf '%s\n' "  warning: live pull-request query failed"
	fi
	printf '%s\n' "Recent merges:"
	if ! gh pr list --state merged --limit 5 --json number,title,mergedAt \
		--template '{{range .}}  #{{.number}} {{.mergedAt}} {{.title}}{{"\n"}}{{end}}'; then
		printf '%s\n' "  warning: merged pull-request query failed"
	fi
	printf '%s\n' "Recent main CI runs:"
	if ! gh run list --branch main --limit 3 \
		--json databaseId,workflowName,status,conclusion,headSha \
		--jq '.[] | "  \(.databaseId) \(.workflowName) \(.status)/\(.conclusion) \(.headSha)"'; then
		printf '%s\n' "  warning: main CI query failed"
	fi
else
	printf '%s\n' "unavailable: GitHub repository or network could not be reached"
fi

section "Start Rule"

printf '%s\n' \
	"Read AGENTS.md and $status_path. Continue an in-progress non-main branch;" \
	"otherwise start from the next ready issue and follow the change-request and Speckit gates."
