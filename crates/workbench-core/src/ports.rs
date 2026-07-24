//! Infrastructure-independent ports used by the orchestration domain.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    AttemptId, CoreError, RequestId, SessionId,
    event::{NewEvent, PersistedEvent},
    routing::RouteCandidate,
    value::{Cursor, NonEmptyText, ProviderId},
};

/// Provider authentication health without credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthenticationStatus {
    Available,
    Unavailable,
    Expired,
    InteractiveRequired,
}

/// Capabilities discoverable before dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCapability {
    Streaming,
    ToolCalling,
    StructuredOutput,
    SessionResume,
    Cancellation,
    Vision,
    Mcp,
    Acp,
}

/// Redacted provider adapter capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub adapter_id: ProviderId,
    pub adapter_version: String,
    pub protocol: String,
    pub authentication: AuthenticationStatus,
    pub capabilities: Vec<ProviderCapability>,
    pub context_window_tokens: Option<u64>,
}

/// Opaque provider session handle that never enters durable event payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionHandle(String);

impl ProviderSessionHandle {
    /// Constructs an opaque handle.
    ///
    /// # Errors
    ///
    /// Returns `provider_unavailable` for an empty handle.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        if value.is_empty() {
            Err(crate::CoreError::new(
                crate::FailureCategory::ProviderUnavailable,
                "provider returned an empty session handle",
            ))
        } else {
            Ok(Self(value))
        }
    }

    /// Provides the handle only to provider adapters.
    #[must_use]
    pub fn expose_to_adapter(&self) -> &str {
        &self.0
    }
}

/// Prompt passed to exactly one selected provider.
#[derive(Debug, Clone)]
pub struct ProviderPrompt {
    pub session_id: SessionId,
    pub attempt_id: AttemptId,
    pub runtime_model: String,
    pub content: NonEmptyText,
}

/// Normalized item emitted by a provider stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOutput {
    Acknowledged {
        provider_request_id: Option<String>,
    },
    Content {
        event_type: String,
        content: NonEmptyText,
    },
    Tool {
        event_type: String,
        content: NonEmptyText,
    },
    Completed {
        summary: String,
    },
}

/// Normalized provider failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub category: crate::FailureCategory,
    pub user_safe_message: String,
    pub definite: bool,
}

/// Streaming provider result.
pub type ProviderStream = BoxStream<'static, Result<ProviderOutput, ProviderFailure>>;

/// Result of a cancellation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationStatus {
    Confirmed,
    Unconfirmed,
}

/// Common provider lifecycle. Vendor-specific details remain behind this port.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn capabilities(&self) -> Result<ProviderCapabilities, CoreError>;
    async fn authentication_status(&self) -> Result<AuthenticationStatus, CoreError>;
    async fn start_session(&self) -> Result<ProviderSessionHandle, ProviderFailure>;
    async fn resume_session(
        &self,
        opaque_handle: &str,
    ) -> Result<ProviderSessionHandle, ProviderFailure>;
    async fn prompt_stream(
        &self,
        handle: &ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) -> Result<ProviderStream, ProviderFailure>;
    async fn cancel(
        &self,
        handle: &ProviderSessionHandle,
        attempt_id: AttemptId,
    ) -> Result<CancellationStatus, CoreError>;
    async fn classify(&self, _input: NonEmptyText) -> Result<RouteCandidate, CoreError> {
        Err(CoreError::new(
            crate::FailureCategory::CapabilityUnavailable,
            "provider does not support coordinator classification",
        ))
    }
}

/// Adapter registry used by preflight and dispatch.
pub trait ProviderRegistry: Send + Sync {
    fn adapter(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>>;
}

/// Append-only event store abstraction.
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Atomically allocates the next sequence and appends one event.
    async fn append(
        &self,
        event: NewEvent,
        occurred_at: OffsetDateTime,
    ) -> Result<PersistedEvent, CoreError>;

    /// Replays events strictly after the cursor.
    async fn load_after(
        &self,
        session_id: SessionId,
        cursor: Cursor,
    ) -> Result<Vec<PersistedEvent>, CoreError>;
}

/// Redacted durable command result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCommand {
    pub method: String,
    pub canonical_parameter_hash: String,
    pub outcome: String,
}

/// Result of atomically committing a state-changing command.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandCommit {
    /// The command was new and its event and outcome were durably committed.
    Applied(PersistedEvent),
    /// The request identifier was already committed for the same command.
    Replayed(RecordedCommand),
}

/// Durable request-id idempotency index.
#[async_trait]
pub trait CommandOutcomeStore: Send + Sync {
    async fn lookup(
        &self,
        session_id: Option<SessionId>,
        request_id: RequestId,
    ) -> Result<Option<RecordedCommand>, CoreError>;

    async fn record(
        &self,
        session_id: Option<SessionId>,
        request_id: RequestId,
        command: RecordedCommand,
    ) -> Result<(), CoreError>;
}

/// Atomic persistence boundary for state-changing session commands.
///
/// Implementations must append the event and record the command outcome in the
/// same transaction. Repeating the same request identifier, method, and
/// parameter hash must return the recorded result without appending an event.
#[async_trait]
pub trait TransactionalCommandStore: Send + Sync {
    async fn commit(
        &self,
        request_id: RequestId,
        command: RecordedCommand,
        event: NewEvent,
        occurred_at: OffsetDateTime,
    ) -> Result<CommandCommit, CoreError>;
}

/// Injectable wall clock for deterministic tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

/// Minimal telemetry boundary; values are bounded and bodies are forbidden.
pub trait Telemetry: Send + Sync {
    fn record_route(&self, selected_rule: &'static str, outcome: &'static str);
    fn record_attempt(&self, outcome: &'static str);
}
