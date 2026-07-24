# Implementation Plan: Create A Thin Replaceable Vs Code Extension Bridge To The

## Overview

Add a small TypeScript VS Code extension under `extensions/workbench-vscode`.
It is a presentation and transport client for the existing local daemon, not a
second orchestration runtime.

## Technical Approach

The extension uses a protocol client with injected transport, a session
controller that tracks the last event cursor and rendered event IDs, and a
webview-free Markdown document panel. A replaceable Mermaid renderer converts
fenced Mermaid blocks to safe VS Code webview content. Commands call the
daemon's existing protocol methods; no provider SDKs or credentials are added.
Unit tests use an in-memory fake transport and fixture events.

## Companion Artifacts

The following optional companion files may be created alongside this
plan to capture additional context:

- `research.md` — background research, prior art, and trade-off notes.
- `data-model.md` — entity and relationship definitions for the feature.
- `contracts/` — interface contracts (OpenAPI, AsyncAPI, CUE schemas).
- `quickstart.md` — step-by-step instructions for running the feature
  locally or in a test environment.
