# Tasks: Create A Thin Replaceable Vs Code Extension Bridge To The

## Task Breakdown

- [x] T001 [P] Scaffold the extension package, activation commands, and typed
  protocol transport boundary under `extensions/workbench-vscode`.
- [x] T002 Implement session attach/replay/live-event deduplication, cursor
  reconnect, prompt/control commands, and bounded error diagnostics.
- [x] T003 Implement Markdown/Mermaid presentation, fake-transport unit tests,
  package validation, and a quickstart for the VS Code workflow.

## Dependencies

The Rust daemon protocol and generated fixtures are the only required external
interface. Tests remain offline; VS Code and Node tooling are needed only to
package or run the extension smoke test.
