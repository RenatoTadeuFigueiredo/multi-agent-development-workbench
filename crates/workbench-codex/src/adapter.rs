use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, future::join_all, stream};
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use uuid::Uuid;
use workbench_core::{
    AttemptId, CoreError, FailureCategory,
    ports::{
        AuthenticationStatus, CancellationStatus, ProviderAdapter, ProviderCapabilities,
        ProviderCapability, ProviderFailure, ProviderOutput, ProviderPrompt, ProviderSessionHandle,
        ProviderStream,
    },
    value::{NonEmptyText, ProviderId},
};

use crate::{
    CODEX_EXEC_JSONL_PROTOCOL, CodexError, CodexErrorKind, CodexLaunchProfile, ShutdownReport,
    process::{CodexProcess, preflight_subscription},
    protocol::Inbound,
};

const PROVIDER_STREAM_QUEUE_DEPTH: usize = 256;
const PROVIDER_STREAM_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const PROCESS_COMMAND_QUEUE_DEPTH: usize = 2;
const MAX_ADAPTER_VERSION_BYTES: usize = 256;
const MAX_CANCELLATION_REAP_RESERVE: Duration = Duration::from_millis(450);

#[derive(Default)]
struct LocalSession {
    setup: AsyncMutex<()>,
    active: Mutex<Option<ActiveAttempt>>,
    pending_cancellations: Mutex<HashSet<AttemptId>>,
}

#[derive(Clone)]
struct ActiveAttempt {
    attempt_id: AttemptId,
    commands: mpsc::Sender<ProcessCommand>,
    cancellation_sent: Arc<AtomicBool>,
}

enum ProcessCommand {
    Cancel {
        attempt_id: AttemptId,
        confirm_by: tokio::time::Instant,
        respond: oneshot::Sender<CancellationStatus>,
    },
    Shutdown {
        respond: oneshot::Sender<ShutdownReport>,
    },
}

struct QueuedOutput {
    item: Result<ProviderOutput, ProviderFailure>,
    _permit: OwnedSemaphorePermit,
}

struct ActiveAttemptGuard {
    sessions: Arc<RwLock<HashMap<String, Arc<LocalSession>>>>,
    handle: String,
    session: Arc<LocalSession>,
    attempt_id: AttemptId,
}

impl Drop for ActiveAttemptGuard {
    fn drop(&mut self) {
        clear_active_attempt(&self.session, self.attempt_id);
        if let Ok(mut pending) = self.session.pending_cancellations.lock() {
            pending.remove(&self.attempt_id);
        }
        if let Ok(mut sessions) = self.sessions.write()
            && sessions
                .get(&self.handle)
                .is_some_and(|session| Arc::ptr_eq(session, &self.session))
        {
            sessions.remove(&self.handle);
        }
    }
}

/// Provider-port implementation backed by one fresh Codex child per attempt.
pub struct CodexProviderAdapter {
    adapter_id: ProviderId,
    adapter_version: String,
    profile: CodexLaunchProfile,
    sessions: Arc<RwLock<HashMap<String, Arc<LocalSession>>>>,
    cancellation_deadline: Duration,
    shutting_down: AtomicBool,
}

impl CodexProviderAdapter {
    /// Runs prompt-free authentication and version identity preflight.
    ///
    /// # Errors
    ///
    /// Returns a redacted core error when the configured CLI is unavailable,
    /// unauthenticated, incompatible, or cannot be reaped.
    pub async fn connect(
        adapter_id: ProviderId,
        expected_adapter_version: String,
        profile: CodexLaunchProfile,
        cancellation_deadline: Duration,
    ) -> Result<Self, CoreError> {
        if cancellation_deadline.is_zero()
            || expected_adapter_version.is_empty()
            || expected_adapter_version.len() > MAX_ADAPTER_VERSION_BYTES
        {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "Codex adapter configuration is invalid",
            ));
        }
        preflight_subscription(&profile, &expected_adapter_version)
            .await
            .map_err(|error| core_error(&error))?;
        Ok(Self {
            adapter_id,
            adapter_version: expected_adapter_version,
            profile,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cancellation_deadline,
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Stops new work and reaps every workspace-owned attempt child.
    pub async fn shutdown(&self) -> ShutdownReport {
        self.shutting_down.store(true, Ordering::Release);
        let sessions = match self.sessions.read() {
            Ok(sessions) => sessions.values().cloned().collect::<Vec<_>>(),
            Err(_) => {
                return ShutdownReport {
                    reaped: false,
                    forced: false,
                };
            }
        };
        let shutdown = async {
            let inspected = join_all(sessions.into_iter().map(|session| async move {
                let _setup = session.setup.lock().await;
                session
                    .active
                    .lock()
                    .map(|active| active.clone())
                    .map_err(|_| ())
            }))
            .await;
            let mut inspection_succeeded = true;
            let active = inspected
                .into_iter()
                .filter_map(|attempt| {
                    if let Ok(attempt) = attempt {
                        attempt
                    } else {
                        inspection_succeeded = false;
                        None
                    }
                })
                .collect::<Vec<_>>();
            let reports = join_all(active.into_iter().map(|attempt| async move {
                let (respond, response) = oneshot::channel();
                if attempt
                    .commands
                    .send(ProcessCommand::Shutdown { respond })
                    .await
                    .is_err()
                {
                    return None;
                }
                response.await.ok()
            }))
            .await;
            reports.into_iter().fold(
                ShutdownReport {
                    reaped: inspection_succeeded,
                    forced: false,
                },
                |mut aggregate, report| {
                    if let Some(report) = report {
                        aggregate.reaped &= report.reaped;
                        aggregate.forced |= report.forced;
                    } else {
                        aggregate.reaped = false;
                    }
                    aggregate
                },
            )
        };
        let mut report = tokio::time::timeout(self.cancellation_deadline, shutdown)
            .await
            .unwrap_or(ShutdownReport {
                reaped: false,
                forced: false,
            });
        match self.sessions.write() {
            Ok(mut sessions) => sessions.clear(),
            Err(_) => report.reaped = false,
        }
        report
    }

    fn require_dispatchable(&self) -> Result<(), ProviderFailure> {
        if self.shutting_down.load(Ordering::Acquire) {
            Err(definite_failure(
                FailureCategory::ProviderUnavailable,
                "Codex provider is shutting down",
            ))
        } else {
            Ok(())
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
impl ProviderAdapter for CodexProviderAdapter {
    async fn capabilities(&self) -> Result<ProviderCapabilities, CoreError> {
        Ok(ProviderCapabilities {
            adapter_id: self.adapter_id.clone(),
            adapter_version: self.adapter_version.clone(),
            protocol: CODEX_EXEC_JSONL_PROTOCOL.to_owned(),
            authentication: if self.shutting_down.load(Ordering::Acquire) {
                AuthenticationStatus::Unavailable
            } else {
                AuthenticationStatus::Available
            },
            capabilities: vec![
                ProviderCapability::Streaming,
                ProviderCapability::Cancellation,
            ],
            context_window_tokens: None,
        })
    }

    async fn authentication_status(&self) -> Result<AuthenticationStatus, CoreError> {
        Ok(if self.shutting_down.load(Ordering::Acquire) {
            AuthenticationStatus::Unavailable
        } else {
            AuthenticationStatus::Available
        })
    }

    async fn start_session(&self) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.require_dispatchable()?;
        let handle = Uuid::now_v7().to_string();
        self.sessions
            .write()
            .map_err(|_| internal_failure(true))?
            .insert(handle.clone(), Arc::new(LocalSession::default()));
        ProviderSessionHandle::new(handle).map_err(|error| ProviderFailure {
            category: error.category(),
            user_safe_message: error.message().to_owned(),
            definite: true,
        })
    }

    async fn resume_session(
        &self,
        _opaque_handle: &str,
    ) -> Result<ProviderSessionHandle, ProviderFailure> {
        Err(definite_failure(
            FailureCategory::CapabilityUnavailable,
            "Codex provider session resume is unavailable",
        ))
    }

    async fn prompt_stream(
        &self,
        handle: &ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) -> Result<ProviderStream, ProviderFailure> {
        self.require_dispatchable()?;
        validate_runtime_model(&prompt.runtime_model)?;
        let session = self.session(handle.expose_to_adapter())?;
        let setup = session.setup.lock().await;
        self.require_dispatchable()?;
        if session
            .active
            .lock()
            .map_err(|_| internal_failure(true))?
            .is_some()
        {
            return Err(definite_failure(
                FailureCategory::ProviderUnavailable,
                "provider session already has an active prompt",
            ));
        }
        let guard = ActiveAttemptGuard {
            sessions: Arc::clone(&self.sessions),
            handle: handle.expose_to_adapter().to_owned(),
            session: Arc::clone(&session),
            attempt_id: prompt.attempt_id,
        };
        if take_pending_cancellation(&session, prompt.attempt_id)? {
            return Err(uncertain_failure());
        }
        let mut process = CodexProcess::spawn_prompt(
            &self.profile,
            &prompt.runtime_model,
            prompt.content.as_str(),
        )
        .await
        .map_err(|error| setup_failure(&error))?;
        if let Err(failure) = self.require_dispatchable() {
            let report = process.shutdown().await;
            return Err(if report.reaped {
                failure
            } else {
                internal_failure(false)
            });
        }
        if take_pending_cancellation(&session, prompt.attempt_id)? {
            let report = process.shutdown().await;
            return Err(if report.reaped {
                uncertain_failure()
            } else {
                internal_failure(false)
            });
        }

        let (commands, receiver) = mpsc::channel(PROCESS_COMMAND_QUEUE_DEPTH);
        let mut active = session.active.lock().map_err(|_| internal_failure(true))?;
        if take_pending_cancellation(&session, prompt.attempt_id)? {
            drop(active);
            let report = process.shutdown().await;
            return Err(if report.reaped {
                uncertain_failure()
            } else {
                internal_failure(false)
            });
        }
        active.replace(ActiveAttempt {
            attempt_id: prompt.attempt_id,
            commands,
            cancellation_sent: Arc::new(AtomicBool::new(false)),
        });
        drop(active);
        let (output, output_receiver) = mpsc::channel(PROVIDER_STREAM_QUEUE_DEPTH);
        let output_budget = Arc::new(Semaphore::new(PROVIDER_STREAM_QUEUE_BYTES));
        tokio::spawn(run_attempt(
            process,
            prompt.attempt_id,
            receiver,
            output,
            Arc::clone(&output_budget),
            guard,
        ));
        drop(setup);
        Ok(stream::unfold(output_receiver, |mut receiver| async {
            receiver.recv().await.map(|queued| (queued.item, receiver))
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
        let active = {
            let active_state = session.active.lock().map_err(|_| {
                CoreError::new(
                    FailureCategory::Internal,
                    "provider session state is unavailable",
                )
            })?;
            let active = active_state
                .as_ref()
                .filter(|active| active.attempt_id == attempt_id)
                .cloned();
            if active.is_none() && active_state.is_none() {
                session
                    .pending_cancellations
                    .lock()
                    .map_err(|_| {
                        CoreError::new(
                            FailureCategory::Internal,
                            "provider session state is unavailable",
                        )
                    })?
                    .insert(attempt_id);
            }
            active
        };
        let Some(active) = active else {
            return Ok(CancellationStatus::Unconfirmed);
        };
        if active.cancellation_sent.swap(true, Ordering::AcqRel) {
            return Ok(CancellationStatus::Unconfirmed);
        }
        let (respond, response) = oneshot::channel();
        let started = tokio::time::Instant::now();
        let deadline = started + self.cancellation_deadline;
        let confirm_by = started + cancellation_confirmation_budget(self.cancellation_deadline);
        if !matches!(
            tokio::time::timeout_at(
                deadline,
                active.commands.send(ProcessCommand::Cancel {
                    attempt_id,
                    confirm_by,
                    respond,
                }),
            )
            .await,
            Ok(Ok(()))
        ) {
            return Ok(CancellationStatus::Unconfirmed);
        }
        match tokio::time::timeout_at(deadline, response).await {
            Ok(Ok(status)) => Ok(status),
            _ => Ok(CancellationStatus::Unconfirmed),
        }
    }
}

async fn run_attempt(
    mut process: CodexProcess,
    attempt_id: AttemptId,
    mut commands: mpsc::Receiver<ProcessCommand>,
    output: mpsc::Sender<QueuedOutput>,
    output_budget: Arc<Semaphore>,
    guard: ActiveAttemptGuard,
) {
    let mut normalization = NormalizationState::default();
    if queue_output(
        &output,
        &output_budget,
        Ok(ProviderOutput::Acknowledged {
            provider_request_id: None,
        }),
    )
    .is_err()
    {
        let _report = process.shutdown().await;
        drop(guard);
        return;
    }
    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(ProcessCommand::Cancel {
                        attempt_id: requested,
                        confirm_by,
                        respond,
                    })
                        if requested == attempt_id =>
                    {
                        let remaining =
                            confirm_by.saturating_duration_since(tokio::time::Instant::now());
                        let status = confirm_cancellation(
                            &mut process,
                            remaining,
                        ).await;
                        let report = process.shutdown().await;
                        let status = if report.reaped { status } else {
                            CancellationStatus::Unconfirmed
                        };
                        drop(guard);
                        let _ignored = respond.send(status);
                        return;
                    }
                    Some(ProcessCommand::Cancel { respond, .. }) => {
                        let _ignored = respond.send(CancellationStatus::Unconfirmed);
                    }
                    Some(ProcessCommand::Shutdown { respond }) => {
                        let report = process.shutdown().await;
                        drop(guard);
                        let _ignored = respond.send(report);
                        return;
                    }
                    None => {
                        let _report = process.shutdown().await;
                        drop(guard);
                        return;
                    }
                }
            }
            inbound = process.next() => {
                let items = match inbound {
                    Ok(Some(inbound)) => normalize(inbound, &mut normalization),
                    Ok(None) | Err(_) => vec![Err(uncertain_failure())],
                };
                for item in items {
                    let terminal = matches!(
                        item,
                        Ok(ProviderOutput::Completed { .. }) | Err(ProviderFailure { .. })
                    );
                    if terminal {
                        let report = process.shutdown().await;
                        let item = if report.reaped {
                            item
                        } else {
                            Err(uncertain_failure())
                        };
                        drop(guard);
                        let _ignored = queue_output(&output, &output_budget, item);
                        return;
                    }
                    if queue_output(&output, &output_budget, item).is_err() {
                        let _report = process.shutdown().await;
                        drop(guard);
                        return;
                    }
                }
            }
        }
    }
}

async fn confirm_cancellation(
    process: &mut CodexProcess,
    deadline: Duration,
) -> CancellationStatus {
    if deadline.is_zero() {
        return CancellationStatus::Unconfirmed;
    }
    let confirmation = async {
        loop {
            let Some(inbound) = process.next().await? else {
                return Ok::<bool, CodexError>(false);
            };
            match inbound {
                Inbound::TurnFailed { cancelled: true } | Inbound::Error { cancelled: true } => {
                    return Ok(true);
                }
                Inbound::TurnCompleted
                | Inbound::TurnFailed { cancelled: false }
                | Inbound::Error { cancelled: false } => return Ok(false),
                Inbound::ThreadStarted
                | Inbound::TurnStarted
                | Inbound::Text { .. }
                | Inbound::ToolStarted { .. }
                | Inbound::Ignored => {}
            }
        }
    };
    match tokio::time::timeout(deadline, confirmation).await {
        Ok(Ok(true)) => CancellationStatus::Confirmed,
        _ => CancellationStatus::Unconfirmed,
    }
}

fn queue_output(
    output: &mpsc::Sender<QueuedOutput>,
    budget: &Arc<Semaphore>,
    item: Result<ProviderOutput, ProviderFailure>,
) -> Result<(), ()> {
    let bytes = normalized_output_bytes(&item).max(1);
    let permits = u32::try_from(bytes).map_err(|_| ())?;
    let permit = Arc::clone(budget)
        .try_acquire_many_owned(permits)
        .map_err(|_| ())?;
    output
        .try_send(QueuedOutput {
            item,
            _permit: permit,
        })
        .map_err(|_| ())
}

fn normalized_output_bytes(item: &Result<ProviderOutput, ProviderFailure>) -> usize {
    match item {
        Ok(ProviderOutput::Acknowledged {
            provider_request_id,
        }) => provider_request_id.as_ref().map_or(0, String::len),
        Ok(
            ProviderOutput::Content {
                event_type,
                content,
            }
            | ProviderOutput::Tool {
                event_type,
                content,
            },
        ) => event_type.len().saturating_add(content.as_str().len()),
        Ok(ProviderOutput::Completed { summary }) => summary.len(),
        Err(failure) => failure.user_safe_message.len(),
    }
}

#[derive(Default)]
struct NormalizationState {
    emitted_text: HashSet<String>,
    emitted_tools: HashSet<String>,
}

fn normalize(
    inbound: Inbound,
    state: &mut NormalizationState,
) -> Vec<Result<ProviderOutput, ProviderFailure>> {
    match inbound {
        Inbound::Text { text } => {
            if text.is_empty() || !state.emitted_text.insert(text.clone()) {
                return Vec::new();
            }
            vec![content_output("assistant_text", text)]
        }
        Inbound::ToolStarted { name } => normalized_tool(name, state).into_iter().collect(),
        Inbound::TurnCompleted => {
            vec![Ok(ProviderOutput::Completed {
                summary: "Codex completed the prompt".to_owned(),
            })]
        }
        Inbound::TurnFailed { cancelled: true } | Inbound::Error { cancelled: true } => {
            vec![Err(uncertain_failure())]
        }
        Inbound::TurnFailed { cancelled: false } | Inbound::Error { cancelled: false } => {
            vec![Err(definite_failure(
                FailureCategory::ProviderUnavailable,
                "Codex reported a provider failure",
            ))]
        }
        Inbound::ThreadStarted | Inbound::TurnStarted | Inbound::Ignored => Vec::new(),
    }
}

fn content_output(event_type: &str, text: String) -> Result<ProviderOutput, ProviderFailure> {
    NonEmptyText::parse(text)
        .map(|content| ProviderOutput::Content {
            event_type: event_type.to_owned(),
            content,
        })
        .map_err(|_| uncertain_failure())
}

fn normalized_tool(
    name: String,
    state: &mut NormalizationState,
) -> Option<Result<ProviderOutput, ProviderFailure>> {
    if !state.emitted_tools.insert(name.clone()) {
        return None;
    }
    Some(
        NonEmptyText::parse(name)
            .map(|content| ProviderOutput::Tool {
                event_type: "read_only_tool_started".to_owned(),
                content,
            })
            .map_err(|_| uncertain_failure()),
    )
}

fn validate_runtime_model(model: &str) -> Result<(), ProviderFailure> {
    if model.is_empty()
        || model.len() > 4_096
        || model.starts_with('-')
        || model.chars().any(char::is_control)
    {
        Err(definite_failure(
            FailureCategory::CapabilityUnavailable,
            "Codex runtime model is invalid",
        ))
    } else {
        Ok(())
    }
}

fn cancellation_confirmation_budget(deadline: Duration) -> Duration {
    let proportional_reserve = deadline / 3;
    let reserve = proportional_reserve.min(MAX_CANCELLATION_REAP_RESERVE);
    deadline.saturating_sub(reserve)
}

fn take_pending_cancellation(
    session: &LocalSession,
    attempt_id: AttemptId,
) -> Result<bool, ProviderFailure> {
    session
        .pending_cancellations
        .lock()
        .map(|mut pending| pending.remove(&attempt_id))
        .map_err(|_| internal_failure(true))
}

fn clear_active_attempt(session: &Arc<LocalSession>, attempt_id: AttemptId) {
    if let Ok(mut active) = session.active.lock()
        && active
            .as_ref()
            .is_some_and(|active| active.attempt_id == attempt_id)
    {
        active.take();
    }
}

fn setup_failure(error: &CodexError) -> ProviderFailure {
    let (category, message) = mapped_failure(error.kind());
    definite_failure(category, message)
}

fn core_error(error: &CodexError) -> CoreError {
    let (category, message) = mapped_failure(error.kind());
    CoreError::new(category, message)
}

fn mapped_failure(kind: CodexErrorKind) -> (FailureCategory, &'static str) {
    match kind {
        CodexErrorKind::InvalidConfiguration => (
            FailureCategory::InvalidRequest,
            "Codex launch configuration is invalid",
        ),
        CodexErrorKind::IncompatibleProtocol | CodexErrorKind::CapabilityUnavailable => (
            FailureCategory::CapabilityUnavailable,
            "Codex provider is incompatible",
        ),
        CodexErrorKind::Timeout => (
            FailureCategory::ProviderTimeout,
            "Codex provider operation timed out",
        ),
        CodexErrorKind::ReapFailed => (
            FailureCategory::Internal,
            "Codex process could not be reaped",
        ),
        CodexErrorKind::SpawnFailed
        | CodexErrorKind::AuthenticationRequired
        | CodexErrorKind::FrameTooLarge
        | CodexErrorKind::InvalidFrame
        | CodexErrorKind::ProtocolViolation
        | CodexErrorKind::TransportClosed
        | CodexErrorKind::ShuttingDown => (
            FailureCategory::ProviderUnavailable,
            "Codex provider is unavailable",
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
        user_safe_message: "Codex outcome could not be confirmed".to_owned(),
        definite: false,
    }
}

fn core_error_from_failure(failure: ProviderFailure) -> CoreError {
    CoreError::new(failure.category, failure.user_safe_message)
}
