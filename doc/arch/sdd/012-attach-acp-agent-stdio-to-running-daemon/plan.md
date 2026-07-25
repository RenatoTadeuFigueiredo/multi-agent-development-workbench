# Implementation Plan: Attach ACP Agent Stdio to Running Daemon

## Approach

1. Add `DaemonSocketBackend` in `workbench-acp-server` that speaks
   `workbench-protocol` NDJSON over a Unix socket (`tokio` + `NdjsonCodec`).
2. Map `BridgeBackend` methods to `initialize`, `session.create`,
   `session.prompt` (+ attach/event drain + approval grant for fake provider),
   and `session.cancel`.
3. Wire `workbench agent stdio` to discover `RuntimePaths` and construct
   `DaemonSocketBackend::connect(&paths.endpoint)`. Fail closed on connect I/O.
4. Keep `InProcessBackend` for offline unit tests and Feature 011 harness.
5. Add `feature_012` acceptance using `LocalDaemonHarness` + socket backend;
   prove second client can `session.list` the ACP-created session.

## Acceptance

- Missing endpoint fails closed quickly.
- Initialize / new / prompt / cancel over socket daemon with fake provider.
- Session list visibility across clients.
- Makefile + inventory (when present) wire Feature 012.

## Residual

Grok terminal WorkbenchBackend remains #33.
