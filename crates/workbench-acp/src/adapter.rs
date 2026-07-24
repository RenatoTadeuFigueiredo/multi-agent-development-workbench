use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use uuid::Uuid;
use workbench_core::{
    AttemptId, CoreError, FailureCategory,
    ports::{
        AuthenticationStatus as CoreAuthenticationStatus, CancellationStatus, ProviderAdapter,
        ProviderCapabilities, ProviderCapability, ProviderFailure, ProviderOutput, ProviderPrompt,
        ProviderSessionHandle, ProviderStream,
    },
    value::{NonEmptyText, ProviderId},
};

use crate::{
    AcpError, AcpErrorKind, AcpSession, AdapterHealth, AuthenticationStatus, CancellationOutcome,
    GrokAcpClient, GrokLaunchProfile, NormalizedUpdate, PromptControl, PromptEvent,
    PromptExecution, ShutdownReport, StopReason, UpdateKind,
};

const PROTOCOL_NAME: &str = "acp/1";
const MAX_MODEL_ID_BYTES: usize = 4_096;
const MAX_ADAPTER_VERSION_BYTES: usize = 256;
const PROVIDER_STREAM_QUEUE_DEPTH: usize = 256;

struct ActivePrompt {
    attempt_id: AttemptId,
    control: PromptControl,
}

struct BoundProviderSession {
    session: AcpSession,
    runtime_model: String,
}

#[derive(Default)]
struct LocalSession {
    provider_session: AsyncMutex<Option<BoundProviderSession>>,
    active: Mutex<Option<ActivePrompt>>,
}

struct ActivePromptGuard {
    session: Arc<LocalSession>,
    attempt_id: AttemptId,
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        clear_active_prompt(&self.session, self.attempt_id);
    }
}

/// Provider-port implementation backed by one supervised Grok Build ACP child.
pub struct GrokProviderAdapter {
    adapter_id: ProviderId,
    adapter_version: String,
    client: Arc<GrokAcpClient>,
    sessions: RwLock<HashMap<String, Arc<LocalSession>>>,
    cancellation_deadline: Duration,
    shutting_down: AtomicBool,
}

impl GrokProviderAdapter {
    /// Connects the supervised child and completes protocol and authentication preflight.
    ///
    /// # Errors
    ///
    /// Returns a redacted core error when the executable cannot be started or
    /// the ACP peer is incompatible.
    pub async fn connect(
        adapter_id: ProviderId,
        expected_adapter_version: String,
        profile: GrokLaunchProfile,
        cancellation_deadline: Duration,
    ) -> Result<Self, CoreError> {
        if cancellation_deadline.is_zero() {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "provider cancellation deadline must be greater than zero",
            ));
        }
        if expected_adapter_version.is_empty()
            || expected_adapter_version.len() > MAX_ADAPTER_VERSION_BYTES
        {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "expected ACP adapter version is invalid",
            ));
        }
        let client = GrokAcpClient::connect(profile)
            .await
            .map_err(|error| core_error(&error))?;
        if client
            .capabilities()
            .agent_version
            .as_deref()
            .is_some_and(|reported| reported != expected_adapter_version)
        {
            if !client.shutdown().await.reaped {
                return Err(CoreError::new(
                    FailureCategory::Internal,
                    "ACP provider process could not be reaped",
                ));
            }
            return Err(CoreError::new(
                FailureCategory::CapabilityUnavailable,
                "ACP provider version does not match the configured pin",
            ));
        }
        Ok(Self {
            adapter_id,
            adapter_version: expected_adapter_version,
            client: Arc::new(client),
            sessions: RwLock::new(HashMap::new()),
            cancellation_deadline,
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Stops new work and reaps the supervised child.
    pub async fn shutdown(&self) -> ShutdownReport {
        self.shutting_down.store(true, Ordering::Release);
        if let Ok(mut sessions) = self.sessions.write() {
            sessions.clear();
        }
        self.client.shutdown().await
    }

    fn authentication(&self) -> CoreAuthenticationStatus {
        match self.client.health() {
            AdapterHealth::Available => {
                map_authentication(self.client.capabilities().authentication)
            }
            AdapterHealth::AuthenticationRequired => CoreAuthenticationStatus::InteractiveRequired,
            AdapterHealth::Starting
            | AdapterHealth::Incompatible
            | AdapterHealth::Unavailable
            | AdapterHealth::Crashed
            | AdapterHealth::ShuttingDown => CoreAuthenticationStatus::Unavailable,
        }
    }

    fn require_dispatchable(&self) -> Result<(), ProviderFailure> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(definite_failure(
                FailureCategory::ProviderUnavailable,
                "ACP provider is shutting down",
            ));
        }
        match self.authentication() {
            CoreAuthenticationStatus::Available => Ok(()),
            CoreAuthenticationStatus::InteractiveRequired => Err(definite_failure(
                FailureCategory::ProviderUnavailable,
                "ACP provider authentication requires user interaction",
            )),
            CoreAuthenticationStatus::Unavailable | CoreAuthenticationStatus::Expired => {
                Err(definite_failure(
                    FailureCategory::ProviderUnavailable,
                    "ACP provider is unavailable",
                ))
            }
        }
    }

    fn session(&self, handle: &str) -> Result<Arc<LocalSession>, ProviderFailure> {
        self.sessions
            .read()
            .map_err(|_| internal_failure(true))?
            .get(handle)
            .cloned()
            .ok_or_else(|| {
                definite_failure(
                    FailureCategory::ProviderUnavailable,
                    "provider session handle is unavailable",
                )
            })
    }
}

#[async_trait]
impl ProviderAdapter for GrokProviderAdapter {
    async fn capabilities(&self) -> Result<ProviderCapabilities, CoreError> {
        let mut capabilities = vec![
            ProviderCapability::Streaming,
            ProviderCapability::Cancellation,
            ProviderCapability::Acp,
        ];
        if self.client.capabilities().load_session {
            capabilities.push(ProviderCapability::SessionResume);
        }
        Ok(ProviderCapabilities {
            adapter_id: self.adapter_id.clone(),
            adapter_version: self.adapter_version.clone(),
            protocol: PROTOCOL_NAME.to_owned(),
            authentication: self.authentication(),
            capabilities,
            context_window_tokens: None,
        })
    }

    async fn authentication_status(&self) -> Result<CoreAuthenticationStatus, CoreError> {
        Ok(self.authentication())
    }

    async fn start_session(&self) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.require_dispatchable()?;
        let local_handle = Uuid::now_v7().to_string();
        self.sessions
            .write()
            .map_err(|_| internal_failure(true))?
            .insert(local_handle.clone(), Arc::new(LocalSession::default()));
        ProviderSessionHandle::new(local_handle).map_err(|error| ProviderFailure {
            category: error.category(),
            user_safe_message: error.message().to_owned(),
            definite: true,
        })
    }

    async fn resume_session(
        &self,
        opaque_handle: &str,
    ) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.require_dispatchable()?;
        self.session(opaque_handle)?;
        ProviderSessionHandle::new(opaque_handle).map_err(|error| ProviderFailure {
            category: error.category(),
            user_safe_message: error.message().to_owned(),
            definite: true,
        })
    }

    async fn prompt_stream(
        &self,
        handle: &ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) -> Result<ProviderStream, ProviderFailure> {
        self.require_dispatchable()?;
        validate_runtime_model(&prompt.runtime_model)?;
        let session = self.session(handle.expose_to_adapter())?;
        let mut provider_session = session.provider_session.lock().await;
        {
            let active = session.active.lock().map_err(|_| internal_failure(true))?;
            if active.is_some() {
                return Err(definite_failure(
                    FailureCategory::ProviderUnavailable,
                    "provider session already has an active prompt",
                ));
            }
        }
        let remote = if let Some(bound) = provider_session.as_ref() {
            if bound.runtime_model != prompt.runtime_model {
                return Err(definite_failure(
                    FailureCategory::CapabilityUnavailable,
                    "provider session runtime model cannot change",
                ));
            }
            bound.session.clone()
        } else {
            let remote = self
                .client
                .new_session(Some(&prompt.runtime_model))
                .await
                .map_err(|error| provider_setup_failure(&error))?;
            provider_session.replace(BoundProviderSession {
                session: remote.clone(),
                runtime_model: prompt.runtime_model.clone(),
            });
            remote
        };
        let execution = self
            .client
            .prompt(&remote, prompt.content.as_str())
            .await
            .map_err(|error| provider_setup_failure(&error))?;
        let control = execution.control();
        session
            .active
            .lock()
            .map_err(|_| internal_failure(true))?
            .replace(ActivePrompt {
                attempt_id: prompt.attempt_id,
                control,
            });
        drop(provider_session);

        let guard = ActivePromptGuard {
            session,
            attempt_id: prompt.attempt_id,
        };
        let (output, receiver) = mpsc::channel(PROVIDER_STREAM_QUEUE_DEPTH);
        tokio::spawn(forward_provider_stream(execution, guard, output));
        Ok(stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|item| (item, receiver))
        })
        .boxed())
    }

    async fn cancel(
        &self,
        handle: &ProviderSessionHandle,
        attempt_id: AttemptId,
    ) -> Result<CancellationStatus, CoreError> {
        let session = self
            .session(handle.expose_to_adapter())
            .map_err(core_error_from_failure)?;
        let control = session
            .active
            .lock()
            .map_err(|_| {
                CoreError::new(
                    FailureCategory::Internal,
                    "provider session state is unavailable",
                )
            })?
            .as_ref()
            .filter(|active| active.attempt_id == attempt_id)
            .map(|active| active.control.clone());
        let Some(control) = control else {
            return Ok(CancellationStatus::Unconfirmed);
        };
        match control.cancel(self.cancellation_deadline).await {
            CancellationOutcome::Confirmed => {
                clear_active_prompt(&session, attempt_id);
                Ok(CancellationStatus::Confirmed)
            }
            CancellationOutcome::Unconfirmed => Ok(CancellationStatus::Unconfirmed),
        }
    }
}

async fn forward_provider_stream(
    mut execution: PromptExecution,
    _guard: ActivePromptGuard,
    output: mpsc::Sender<Result<ProviderOutput, ProviderFailure>>,
) {
    let mut consumer_open = true;
    loop {
        let item = match execution.next().await {
            Ok(Some(PromptEvent::Update(update))) => {
                let Some(item) = normalize_update(update) else {
                    continue;
                };
                item
            }
            Ok(Some(PromptEvent::Finished(outcome))) => normalize_outcome(outcome.stop_reason),
            Ok(None) => return,
            Err(_) => Err(uncertain_failure()),
        };
        let terminal = matches!(
            &item,
            Ok(ProviderOutput::Completed { .. }) | Err(ProviderFailure { .. })
        );
        if consumer_open && output.send(item).await.is_err() {
            consumer_open = false;
        }
        if terminal {
            return;
        }
    }
}

fn normalize_update(update: NormalizedUpdate) -> Option<Result<ProviderOutput, ProviderFailure>> {
    match update.kind {
        UpdateKind::Acknowledged => Some(Ok(ProviderOutput::Acknowledged {
            provider_request_id: None,
        })),
        UpdateKind::AgentMessage => normalized_content("agent_message_chunk", update.content),
        UpdateKind::AgentThought => None,
        UpdateKind::Plan => normalized_content("plan", update.content),
        UpdateKind::ToolCall => normalized_tool("tool_call", update.content),
        UpdateKind::ToolCallUpdate => normalized_tool("tool_call_update", update.content),
    }
}

fn normalized_content(
    event_type: &str,
    value: Option<Value>,
) -> Option<Result<ProviderOutput, ProviderFailure>> {
    normalized_text(value).map(|content| {
        content.map(|content| ProviderOutput::Content {
            event_type: event_type.to_owned(),
            content,
        })
    })
}

fn normalized_tool(
    event_type: &str,
    value: Option<Value>,
) -> Option<Result<ProviderOutput, ProviderFailure>> {
    normalized_text(value).map(|content| {
        content.map(|content| ProviderOutput::Tool {
            event_type: event_type.to_owned(),
            content,
        })
    })
}

fn normalized_text(value: Option<Value>) -> Option<Result<NonEmptyText, ProviderFailure>> {
    let value = value?;
    if let Some(text) = value
        .as_object()
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
    {
        if text.is_empty() {
            return None;
        }
        return Some(NonEmptyText::parse(text).map_err(|_| internal_failure(false)));
    }
    Some(
        serde_json::to_string(&value)
            .map_err(|_| internal_failure(false))
            .and_then(|text| NonEmptyText::parse(text).map_err(|_| internal_failure(false))),
    )
}

fn normalize_outcome(stop_reason: StopReason) -> Result<ProviderOutput, ProviderFailure> {
    let summary = match stop_reason {
        StopReason::EndTurn => "provider completed the prompt",
        StopReason::MaxTokens => "provider reached its token limit",
        StopReason::MaxTurnRequests => "provider reached its turn request limit",
        StopReason::Refusal => "provider ended the prompt with a refusal",
        StopReason::Cancelled => return Err(cancelled_stream_failure()),
    };
    Ok(ProviderOutput::Completed {
        summary: summary.to_owned(),
    })
}

fn validate_runtime_model(model: &str) -> Result<(), ProviderFailure> {
    if model.is_empty() || model.len() > MAX_MODEL_ID_BYTES {
        Err(definite_failure(
            FailureCategory::CapabilityUnavailable,
            "provider runtime model is invalid",
        ))
    } else {
        Ok(())
    }
}

fn map_authentication(status: AuthenticationStatus) -> CoreAuthenticationStatus {
    match status {
        AuthenticationStatus::Available => CoreAuthenticationStatus::Available,
        AuthenticationStatus::InteractiveRequired => CoreAuthenticationStatus::InteractiveRequired,
        AuthenticationStatus::Unavailable => CoreAuthenticationStatus::Unavailable,
    }
}

fn provider_setup_failure(error: &AcpError) -> ProviderFailure {
    let (category, message) = mapped_failure(error.kind());
    definite_failure(category, message)
}

fn core_error(error: &AcpError) -> CoreError {
    let (category, message) = mapped_failure(error.kind());
    CoreError::new(category, message)
}

fn mapped_failure(kind: AcpErrorKind) -> (FailureCategory, &'static str) {
    match kind {
        AcpErrorKind::InvalidConfiguration => (
            FailureCategory::InvalidRequest,
            "ACP launch configuration is invalid",
        ),
        AcpErrorKind::IncompatibleProtocol | AcpErrorKind::CapabilityUnavailable => (
            FailureCategory::CapabilityUnavailable,
            "ACP provider is incompatible",
        ),
        AcpErrorKind::Timeout => (
            FailureCategory::ProviderTimeout,
            "ACP provider operation timed out",
        ),
        AcpErrorKind::SpawnFailed
        | AcpErrorKind::AuthenticationRequired
        | AcpErrorKind::RequestFailed
        | AcpErrorKind::TransportClosed
        | AcpErrorKind::ShuttingDown
        | AcpErrorKind::FrameTooLarge
        | AcpErrorKind::InvalidFrame
        | AcpErrorKind::ProtocolViolation => (
            FailureCategory::ProviderUnavailable,
            "ACP provider is unavailable",
        ),
        AcpErrorKind::ReapFailed => (
            FailureCategory::Internal,
            "ACP provider process could not be reaped",
        ),
    }
}

fn definite_failure(category: FailureCategory, message: impl Into<String>) -> ProviderFailure {
    ProviderFailure {
        category,
        user_safe_message: message.into(),
        definite: true,
    }
}

fn internal_failure(definite: bool) -> ProviderFailure {
    ProviderFailure {
        category: FailureCategory::Internal,
        user_safe_message: "provider adapter state is unavailable".to_owned(),
        definite,
    }
}

fn uncertain_failure() -> ProviderFailure {
    ProviderFailure {
        category: FailureCategory::OutcomeUnknown,
        user_safe_message: "provider outcome is unknown after prompt dispatch".to_owned(),
        definite: false,
    }
}

fn cancelled_stream_failure() -> ProviderFailure {
    ProviderFailure {
        category: FailureCategory::OutcomeUnknown,
        user_safe_message: "provider prompt ended through cancellation".to_owned(),
        definite: false,
    }
}

fn core_error_from_failure(failure: ProviderFailure) -> CoreError {
    CoreError::new(failure.category, failure.user_safe_message)
}

fn clear_active_prompt(session: &LocalSession, attempt_id: AttemptId) {
    if let Ok(mut active) = session.active.lock()
        && active
            .as_ref()
            .is_some_and(|active| active.attempt_id == attempt_id)
    {
        active.take();
    }
}
