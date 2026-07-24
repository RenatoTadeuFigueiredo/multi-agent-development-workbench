# Workbench VS Code Bridge

This extension is a thin presentation client for the local Workbench daemon. It
does not contain routing, provider, policy, or persistence logic.

## Development

```bash
npm install
npm run compile
npm test
```

The bridge discovers the daemon socket from `XDG_RUNTIME_DIR` on Linux and
`TMPDIR` plus the current UID on macOS. Set the absolute `workbench.endpoint`
setting only when the daemon uses a custom runtime directory.
Run `workbench config lock` and `workbench daemon` from the repository before
using **Workbench: Attach Session**. The extension keeps the active event view
in memory and reconnects from its last event cursor through the daemon API.
The virtual Markdown document is displayed through VS Code's native Markdown
preview, including its Mermaid support; no transcript is written by the bridge.
