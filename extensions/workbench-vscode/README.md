# Workbench VS Code Bridge

This extension is a thin presentation client for the local Workbench daemon. It
does not contain routing, provider, policy, or persistence logic.

## Development

```bash
npm install
npm run compile
npm test
```

The bridge derives the daemon socket from the canonical current-workspace path:
`XDG_RUNTIME_DIR/workbench/<workspace-id>.sock` on Linux and
`TMPDIR/workbench-<uid>/<workspace-id>.sock` on macOS. Set the absolute,
workspace-scoped `workbench.endpoint` only when the daemon uses a custom
runtime directory or when no workspace is open.
Run `workbench config lock` and `workbench daemon` from the repository before
using the extension commands. **Workbench: New Session** creates a persistent
session and attaches it. **Workbench: Select Session** lists metadata-only
summaries from the daemon configured for the current workspace and attaches the
selection. **Workbench: Attach Session** remains available when you already
know an ID.

The picker never lists sessions from another endpoint or workspace. It shows
only a session ID, lifecycle state, creation time, and terminal time when one
exists; prompts, events, configuration, and provider content are not part of
the discovery response. The extension keeps the active event view in memory
and reconnects from its last event cursor through the daemon API.
The virtual Markdown document is displayed through VS Code's native Markdown
preview, including its Mermaid support; no transcript is written by the bridge.
