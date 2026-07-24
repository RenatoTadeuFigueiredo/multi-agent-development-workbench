# Local Service-Level Objectives

## Scope

These objectives cover the feature 001 daemon with deterministic fake adapters
on the pinned macOS and Linux CI reference runners. Provider latency, editor
rendering, remote access, and paid-model availability are outside this
measurement boundary.

## SLO-001: Durable Correctness

| SLI | Objective | Measurement |
|---|---|---|
| Accepted-command durability | 100% of acknowledged inputs and controls survive forced restart | Crash-point integration suite |
| Slow-client isolation | Zero blocked healthy clients and zero lost committed events | Queue-limit stress test |
| Sensitive plaintext leakage | Zero prompt or provider-output matches in database, WAL, logs, and crash fixtures | Byte-level inspection tests |
| Default external usage | Zero network requests and zero paid quota | Network-denial test harness |

## SLO-002: Local Responsiveness

| SLI | Objective | Measurement |
|---|---|---|
| Routing-plan latency | p95 at or below 100 ms after durable input append | 1,000 deterministic local routes |
| Event fan-out latency | p95 at or below 100 ms for a healthy attached client | Timestamp at append and client receipt |
| Replay throughput | 10,000 events replayed within 2 seconds with no ordering loss | Local encrypted SQLite fixture |
| Control acknowledgement | p95 at or below 100 ms, excluding provider safe-point completion | Protocol contract benchmark |
| Cancellation resolution | Confirmation or `outcome_unknown` within 5 seconds | Fake responsive and unresponsive adapters |

The fan-out benchmark measures from the durable event's `occurred_at` timestamp
through receipt by the attached IPC client. Because the timestamp is assigned
immediately before the storage transaction, this is a conservative window that
also includes persistence and publication.

## Error Budget Policy

The release gate evaluates every objective on both platforms. A failed
correctness, durability, security, or five-second cancellation objective blocks
release. A latency regression blocks release after one clean-run retry rules
out runner noise; targets may change only through a reviewed specification
update with recorded before-and-after measurements.
