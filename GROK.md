# multi-agent-development-workbench — Grok Build Guide

Guidance for Grok Build / Grok Code when working in this repository.
`AGENTS.md` is the canonical, tool-agnostic project map; this file adds
Grok-specific notes. When the two overlap, `AGENTS.md` wins.

Prune as you go: when the code moves on, refresh what drifted and delete dead
sections and stale path references — keep this file lean, do not only append to
it.

## Project

Multi-Agent Development Workbench is managed with Speckit. Treat `doc/arch/`
as the source of truth: read it before editing code and change specifications
first whenever behavior changes. Drive the protocol with the installed
`speckit` binary; never invent control-plane state.

## Architecture

Authoritative layout (see `AGENTS.md` for the annotated tree):

```
doc/arch/
├── adr/              # decision records — read these first
├── schemas/          # CUE schemas
├── specs/features/   # Gherkin behavior specs
├── architecture/     # C4 model + diagrams
└── sdd/              # active-feature working dirs (NNN-slug)
```

## Commands

```
speckit status      # active feature + current workflow phase
speckit next        # the recommended next command
speckit validate    # validate the doc/arch corpus before committing
speckit context score   # score AGENTS.md / CLAUDE.md / GROK.md / llms.txt
make check          # current non-mutating validation and test gate
```

## Grok Build integration

- Always invoke the **installed `speckit` binary** for status, next, validate,
  guard, and scoring — do not approximate the control plane by reading or
  writing `doc/.specify/` databases by hand.
- Prefer project-local facts from `AGENTS.md` over generic training knowledge
  when they conflict.
- Use Grok Build tools for local edits and shell; keep network, secrets, and
  remote hosts behind the project's declared ports and policies.
- Root docs `README.md`, `AGENTS.md`, `CLAUDE.md`, and `GROK.md` are in derived
  guard scope so maintenance edits are allowed without opening the whole tree.

## Conventions

- Respect the guard policy (`[guard]` in `doc/arch/speckit.toml`). If a write is
  denied, the target is outside the active spec scope — do not disable the
  guard; adjust the scope or revise the plan.
- Follow the workflow: constitution → specify → clarify → plan → tasks →
  analyze → implement → validate.
- Keep `AGENTS.md`, `GROK.md`, `README.md`, and the `doc/arch` corpus in sync
  with code; persist all artifacts in English.
- Keep Grok ACP behavior in `workbench-acp` and Claude stream JSON behavior in
  `workbench-claude`; neither provider protocol belongs in the domain.

## Spec-first protocol

The deterministic loop — never skip a step, never guess:

- Before any work, run `speckit status`, then `speckit next`; execute exactly
  the phase it points to — no more, no less.
- `speckit validate` must be green before every commit.
- The source of truth is `doc/arch`: spec > code > assumption. Never guess —
  read the spec.
- The guard denies writes outside the active scope; never bypass it and never
  edit `doc/.specify/` state by hand.
- A red `validate`/`check` blocks everything: fix the artifact, never the rule.

### Git workflow

- Start product changes from a tracked issue on a feature branch from `main`.
- Present the reviewed diff and wait for human approval before opening a pull
  request.
- Commit after every logical change; never batch unrelated edits into a single
  commit. The git history is part of the spec corpus — keep it legible.
- Write Angular Conventional Commit headers: `<type>(<scope>): <subject>`, with
  a short, objective subject (keep the header line at most 72 characters).
- Never bypass speckit gates (guard, `validate`, `verify`) to force a commit
  through — fix the spec or adjust the scope instead.
- Never add AI attribution — AI co-author trailers, "generated with" notices, or
  assistant session links — to code, docs, or commit messages; this is a
  compliance mandate (ADR-0026).
- If such attribution slips in, strip it with an interactive rebase before you
  push the branch.
