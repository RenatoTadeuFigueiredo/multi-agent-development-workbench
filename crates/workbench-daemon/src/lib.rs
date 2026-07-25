//! Local Workbench daemon composition.

#![forbid(unsafe_code)]

pub mod application;
pub mod ipc;
pub mod providers;
pub mod runtime;
pub mod runtime_paths;
pub mod startup;
mod storage_backend;
mod subscription;
pub mod telemetry;
mod workflow_exec;

pub use application::{Application, ClientContext, DispatchResult, FakeBehavior};
pub use runtime::{DaemonRuntime, ShutdownHandle};
pub use startup::StartupConfiguration;
pub use telemetry::{
    BoundedTelemetry, ExternalTelemetryExport, RouteRule, TelemetryError, TelemetryOutcome,
    TelemetrySnapshot,
};
