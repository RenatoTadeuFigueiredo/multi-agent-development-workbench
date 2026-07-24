//! Deterministic, offline adapters and contract fixtures.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod client;
pub mod clock;
pub mod contracts;
pub mod network;
pub mod provider;
pub mod telemetry;
pub mod tool;

pub use clock::FakeClock;
pub use network::{DenyNetwork, NetworkDenied};
pub use provider::{
    CoordinatorBehavior, FakeProvider, FakeProviderBuilder, FakeProviderRegistry,
    ProviderCallCounts, ResumeBehavior, SetupBehavior, StreamBehavior,
};
pub use telemetry::{TelemetryRecord, TelemetrySink};
pub use tool::{FakeTool, ToolCall, ToolOutcome};
pub use workbench_storage::MemoryKeyStore as FakeKeyStore;
