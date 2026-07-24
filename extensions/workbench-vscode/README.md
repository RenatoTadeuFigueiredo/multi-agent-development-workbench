# Workbench VS Code Bridge

This extension is a thin presentation client for the local Workbench daemon. It
does not contain routing, provider, policy, or persistence logic.

## Development

```bash
npm install
npm run compile
npm test
```

Set `workbench.endpoint` when the daemon socket is not at the default path.
Run `workbench config lock` and `workbench daemon` from the repository before
using **Workbench: Attach Session**. The extension keeps the active event view
in memory and reconnects from its last event cursor through the daemon API.
