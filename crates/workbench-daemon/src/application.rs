use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tracing::{info, warn};
use uuid::Uuid;
use workbench_config::{
    ConfigurationSnapshot, WorkbenchConfiguration, WorkbenchLock,
    model::{
        ApprovalMode, Capability, DefaultToolMode, EffectClass as ConfigEffectClass, ProviderType,
    },
    preflight::{
        Authentication, ProviderCapabilities as ConfigProviderCapabilities, ProviderOperation,
        ResolvedModel, resolve_role,
    },
};
use workbench_core::{
    AttemptId, SessionId,
    attempt::EffectClass,
    policy::{PermissionMode, PolicyLayer, PolicySource, protect_effect, resolve_tool_policy},
    ports::{
        CancellationStatus, ProviderAdapter, ProviderFailure, ProviderOutput, ProviderPrompt,
        ProviderRegistry, ProviderSessionHandle, Telemetry,
    },
    routing::{
        OrderedRouter, PermissionScope, Risk, RouteCandidate, RouteContext, RouteDestination,
        RoutingInputs, RoutingOutcome, RoutingPlan, SelectedRule,
    },
    value::{DataSourceId, ModelAlias, NonEmptyText, ProviderId, RoleId, ToolId},
};
use workbench_protocol::{
    ClientCommand, Command, ErrorCode, EventKind, ProtocolError, ServerReply, SessionEvent,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, DeleteParams,
        ExportParams, ListSessionsParams, PromptParams, ReconciliationParams,
        ReconciliationResolution, RedirectParams,
    },
    response::{
        AdapterHealth, AdapterStatus, ApprovalResult, AttachSessionResult, Control, ControlResult,
        CreateSessionResult, DeleteResult, DeleteState, ExportFormat, ExportResult,
        InitializeResult, KeyStoreStatus, ListSessionsResult, MigrationStatus, PromptResult,
        ProtocolVersion, ReadyState, ReconciliationResult, SessionResult, SessionState,
        SessionSummary, StatusResult,
    },
};
use workbench_storage::{
    CommandEventOutcome, CommandEventsOutcome, CommandOutcome, CreateSession, DeletionSummary,
    EventInput, ExportCommand, KeyStore, MemoryKeyStore, SqliteStorage, StorageError,
    StoredSession, recipient_fingerprints,
};

use crate::{
    startup::StartupConfiguration,
    storage_backend::{LockedStorage, StorageBackend},
    subscription::{SessionSubscription, SubscriptionHub},
    telemetry::BoundedTelemetry,
};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RETENTION_DELETIONS_PER_PASS: usize = 64;
const DURABLE_FAILURE_FIELD: &str = "_workbench_failure";

#[derive(Clone, Copy, Debug)]
pub struct FakeBehavior {
    pub response_delay: Duration,
    pub confirms_cancellation: bool,
    pub cancellation_deadline: Duration,
}

impl Default for FakeBehavior {
    fn default() -> Self {
        Self {
            response_delay: Duration::from_millis(10),
            confirms_cancellation: true,
            cancellation_deadline: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClientContext {
    pub uid: u32,
    pub client_name: String,
}

impl ClientContext {
    #[must_use]
    pub fn actor(&self) -> String {
        format!("local-user:{}", self.uid)
    }
}

pub(crate) struct DispatchResult {
    pub reply: ServerReply<Value>,
    pub subscription: Option<SessionSubscription>,
}

struct CommandSuccess {
    result: Value,
    subscription: Option<SessionSubscription>,
}

struct ActiveExecution {
    attempt_id: Uuid,
    request_id: Uuid,
    backend: ActiveBackend,
    cancel_started: AtomicBool,
}

enum ActiveBackend {
    Fake,
    Provider {
        adapter: Arc<dyn ProviderAdapter>,
        handle: ProviderSessionHandle,
    },
}

impl ActiveExecution {
    fn fake(attempt_id: Uuid, request_id: Uuid) -> Self {
        Self {
            attempt_id,
            request_id,
            backend: ActiveBackend::Fake,
            cancel_started: AtomicBool::new(false),
        }
    }

    fn provider(
        attempt_id: Uuid,
        request_id: Uuid,
        adapter: Arc<dyn ProviderAdapter>,
        handle: ProviderSessionHandle,
    ) -> Self {
        Self {
            attempt_id,
            request_id,
            backend: ActiveBackend::Provider { adapter, handle },
            cancel_started: AtomicBool::new(false),
        }
    }

    const fn is_fake(&self) -> bool {
        matches!(self.backend, ActiveBackend::Fake)
    }
}

#[derive(Default)]
struct EmptyProviderRegistry;

impl ProviderRegistry for EmptyProviderRegistry {
    fn adapter(&self, _provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        None
    }
}

enum RouteDecision {
    Selected {
        plan: RoutingPlan,
        permission: DefaultToolMode,
    },
    Clarification {
        reason: String,
        selected_rule: &'static str,
    },
    CapabilityUnavailable {
        selected_rule: &'static str,
    },
}

struct ProviderExecutionContext {
    provider: ProviderId,
    runtime_model: String,
    prompt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderOutputState {
    Streaming,
    CancellationPending,
    Terminal,
}

impl ProviderOutputState {
    const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

#[derive(Debug)]
struct FoldedSession {
    state: SessionState,
    last_sequence: u64,
    pending_approval_id: Option<Uuid>,
    pending_request_id: Option<Uuid>,
    uncertain_attempt_id: Option<Uuid>,
    approval_decisions: HashMap<Uuid, ApprovalDecision>,
}

pub struct Application {
    storage: Arc<dyn StorageBackend>,
    startup: StartupConfiguration,
    lifecycle_gate: AsyncRwLock<()>,
    shutting_down: AtomicBool,
    session_locks: AsyncMutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>,
    creation_lock: AsyncMutex<()>,
    session_configs: Mutex<HashMap<Uuid, WorkbenchConfiguration>>,
    pinned_locks: Mutex<HashMap<Uuid, WorkbenchLock>>,
    known_sessions: Mutex<HashSet<Uuid>>,
    active: AsyncMutex<HashMap<Uuid, Arc<ActiveExecution>>>,
    subscriptions: Arc<SubscriptionHub>,
    telemetry: Arc<dyn Telemetry>,
    providers: Arc<dyn ProviderRegistry>,
    provider_catalog: BTreeMap<String, ConfigProviderCapabilities>,
    fake: FakeBehavior,
    #[cfg(test)]
    fail_next_command_commit: AtomicBool,
    #[cfg(test)]
    fail_next_deletion_request: AtomicBool,
    #[cfg(test)]
    history_replays: AtomicUsize,
}

impl Application {
    pub fn new<K: KeyStore + 'static>(
        storage: SqliteStorage<K>,
        startup: StartupConfiguration,
        fake: FakeBehavior,
    ) -> Arc<Self> {
        Self::new_with_telemetry(
            storage,
            startup,
            fake,
            Arc::new(BoundedTelemetry::default()),
        )
    }

    /// Creates an application with externally composed provider adapters and
    /// their already validated preflight capabilities.
    pub fn new_with_providers<K: KeyStore + 'static>(
        storage: SqliteStorage<K>,
        startup: StartupConfiguration,
        fake: FakeBehavior,
        providers: Arc<dyn ProviderRegistry>,
        provider_catalog: BTreeMap<String, ConfigProviderCapabilities>,
    ) -> Arc<Self> {
        Self::new_with_providers_and_telemetry(
            storage,
            startup,
            fake,
            Arc::new(BoundedTelemetry::default()),
            providers,
            provider_catalog,
        )
    }

    pub(crate) fn new_with_telemetry<K: KeyStore + 'static>(
        storage: SqliteStorage<K>,
        startup: StartupConfiguration,
        fake: FakeBehavior,
        telemetry: Arc<dyn Telemetry>,
    ) -> Arc<Self> {
        Self::compose(
            storage,
            startup,
            fake,
            telemetry,
            Arc::new(EmptyProviderRegistry),
            BTreeMap::new(),
        )
    }

    /// Creates an application with externally composed providers and an
    /// injectable bounded telemetry sink.
    pub fn new_with_providers_and_telemetry<K: KeyStore + 'static>(
        storage: SqliteStorage<K>,
        startup: StartupConfiguration,
        fake: FakeBehavior,
        telemetry: Arc<dyn Telemetry>,
        providers: Arc<dyn ProviderRegistry>,
        provider_catalog: BTreeMap<String, ConfigProviderCapabilities>,
    ) -> Arc<Self> {
        Self::compose(
            storage,
            startup,
            fake,
            telemetry,
            providers,
            provider_catalog,
        )
    }

    fn compose<K: KeyStore + 'static>(
        storage: SqliteStorage<K>,
        startup: StartupConfiguration,
        fake: FakeBehavior,
        telemetry: Arc<dyn Telemetry>,
        providers: Arc<dyn ProviderRegistry>,
        provider_catalog: BTreeMap<String, ConfigProviderCapabilities>,
    ) -> Arc<Self> {
        Arc::new(Self {
            storage: Arc::new(LockedStorage::new(storage)),
            startup,
            lifecycle_gate: AsyncRwLock::new(()),
            shutting_down: AtomicBool::new(false),
            session_locks: AsyncMutex::new(HashMap::new()),
            creation_lock: AsyncMutex::new(()),
            session_configs: Mutex::new(HashMap::new()),
            pinned_locks: Mutex::new(HashMap::new()),
            known_sessions: Mutex::new(HashSet::new()),
            active: AsyncMutex::new(HashMap::new()),
            subscriptions: Arc::new(SubscriptionHub::default()),
            telemetry,
            providers,
            provider_catalog,
            fake,
            #[cfg(test)]
            fail_next_command_commit: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_deletion_request: AtomicBool::new(false),
            #[cfg(test)]
            history_replays: AtomicUsize::new(0),
        })
    }

    /// Creates a persistent-semantics application over an encrypted in-memory store.
    ///
    /// # Errors
    ///
    /// Returns an error when the encrypted storage boundary cannot initialize.
    pub fn in_memory(
        startup: StartupConfiguration,
        fake: FakeBehavior,
    ) -> Result<Arc<Self>, StorageError> {
        Ok(Self::new(
            SqliteStorage::open_in_memory(MemoryKeyStore::new())?,
            startup,
            fake,
        ))
    }

    #[cfg(test)]
    fn in_memory_with_telemetry(
        startup: StartupConfiguration,
        fake: FakeBehavior,
        telemetry: Arc<dyn Telemetry>,
    ) -> Result<Arc<Self>, StorageError> {
        Ok(Self::new_with_telemetry(
            SqliteStorage::open_in_memory(MemoryKeyStore::new())?,
            startup,
            fake,
            telemetry,
        ))
    }

    /// Reconciles interrupted creations, attempts, controls, exports, and deletions.
    ///
    /// # Errors
    ///
    /// Returns a redacted storage error when any recovery step fails.
    pub fn recover(&self) -> Result<(), StorageError> {
        self.storage.resume_session_creations()?;
        let recovered_exports = self.storage.resume_exports()?;
        for event in recovered_exports.iter().cloned() {
            self.subscriptions.publish(
                &protocol_event(event).map_err(|_| StorageError::StorageUnavailable(None))?,
            );
        }
        let resumed = self.storage.resume_deletions()?;
        let sessions = self.storage.load_sessions()?;
        for session in &sessions {
            self.hydrate_stored_session(session)?;
        }
        let recovered_controls = self.recover_incomplete_controls(&sessions)?;
        let uncertain = self
            .storage
            .recover_uncertain_attempts(OffsetDateTime::now_utc())?;
        for _ in &uncertain {
            self.telemetry.record_attempt("outcome_unknown");
        }
        drop(sessions);
        info!(
            recovered_exports = recovered_exports.len(),
            resumed_deletions = resumed.len(),
            recovered_controls,
            recovered_attempts = uncertain.len(),
            "daemon recovery completed"
        );
        Ok(())
    }

    /// Applies one bounded pass of per-session pinned retention.
    ///
    /// # Errors
    ///
    /// Returns a redacted storage error if a pinned session cannot be loaded or
    /// its deletion journal cannot be completed.
    pub async fn run_maintenance(&self, now: OffsetDateTime) -> Result<usize, StorageError> {
        let sessions = self.storage.load_sessions()?;
        let mut candidates = Vec::with_capacity(MAX_RETENTION_DELETIONS_PER_PASS);
        for session in &sessions {
            self.hydrate_stored_session(session)?;
            if self.retention_due(session, now)? {
                candidates.push(session.session_id);
                if candidates.len() == MAX_RETENTION_DELETIONS_PER_PASS {
                    break;
                }
            }
        }
        drop(sessions);

        let mut deleted = 0;
        for session_id in candidates {
            let lock = self.session_lock(session_id).await;
            let _guard = lock.lock().await;
            let stored = match self.storage.load_session(session_id) {
                Ok(stored) => stored,
                Err(StorageError::SessionNotFound) => continue,
                Err(error) => return Err(error),
            };
            self.hydrate_stored_session(&stored)?;
            if !self.retention_due(&stored, now)? {
                continue;
            }
            match self.storage.ensure_session_deletable(session_id) {
                Ok(()) => {}
                Err(StorageError::InvalidInput(_)) => continue,
                Err(error) => return Err(error),
            }

            self.request_deletion(
                session_id,
                Uuid::now_v7(),
                Uuid::now_v7(),
                now,
                "system:retention",
            )?;
            self.active.lock().await.remove(&session_id);
            self.evict_session(session_id)?;
            self.subscriptions
                .purge_session(session_id)
                .map_err(|_| StorageError::StorageUnavailable(None))?;
            deleted += 1;
        }
        Ok(deleted)
    }

    fn retention_due(
        &self,
        session: &StoredSession,
        now: OffsetDateTime,
    ) -> Result<bool, StorageError> {
        if !matches!(
            session.state.as_str(),
            "completed" | "failed" | "cancelled" | "abandoned"
        ) {
            return Ok(false);
        }
        let Some(terminal_at) = session.terminal_at else {
            return Ok(false);
        };
        let retention_days = self
            .session_configs
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .get(&session.session_id)
            .ok_or(StorageError::StorageUnavailable(None))?
            .storage
            .retention_days;
        let Some(days) = retention_days else {
            return Ok(false);
        };
        if days == 0 {
            return Err(StorageError::InvalidInput(
                "retention must be at least one day",
            ));
        }
        Ok(terminal_at <= now - time::Duration::days(i64::from(days)))
    }

    fn hydrate_stored_session(&self, stored: &StoredSession) -> Result<(), StorageError> {
        let snapshot: ConfigurationSnapshot =
            serde_json::from_value(stored.configuration_snapshot.clone())?;
        if snapshot.schema_version != 1 {
            return Err(StorageError::InvalidInput(
                "persisted configuration snapshot version is unsupported",
            ));
        }
        let configuration: WorkbenchConfiguration =
            serde_json::from_value(snapshot.configuration.clone())?;
        let session_lock: WorkbenchLock = serde_json::from_value(stored.lock_snapshot.clone())?;
        session_lock
            .verify()
            .map_err(|_| StorageError::InvalidInput("persisted session lock is invalid"))?;
        if session_lock.configuration.resolved_hash != snapshot.content_hash {
            return Err(StorageError::AuthenticationFailed);
        }
        self.session_configs
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .insert(stored.session_id, configuration);
        self.pinned_locks
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .insert(stored.session_id, session_lock);
        self.known_sessions
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .insert(stored.session_id);
        Ok(())
    }

    fn recover_incomplete_controls(
        &self,
        sessions: &[StoredSession],
    ) -> Result<usize, StorageError> {
        let mut recovered = 0;
        for session in sessions {
            let history = self.storage.replay(session.session_id, 0)?;
            let Some(last) = history.last() else {
                continue;
            };
            match last.kind.as_str() {
                "pause_requested" => {
                    self.storage.append_event(&EventInput {
                        event_id: Uuid::now_v7(),
                        session_id: session.session_id,
                        occurred_at: OffsetDateTime::now_utc(),
                        kind: "session_paused".to_owned(),
                        causation_request_id: last.causation_request_id,
                        attempt_id: None,
                        effect_class: None,
                        payload: last.payload.clone(),
                    })?;
                    recovered += 1;
                }
                "cancel_requested" => {
                    let attempt_id = latest_started_attempt(&history);
                    let now = OffsetDateTime::now_utc();
                    let mut events = Vec::new();
                    if attempt_id.is_none() {
                        events.push(EventInput {
                            event_id: Uuid::now_v7(),
                            session_id: session.session_id,
                            occurred_at: now,
                            kind: "cancel_confirmed".to_owned(),
                            causation_request_id: last.causation_request_id,
                            attempt_id,
                            effect_class: None,
                            payload: json!({
                                "control_id": value_uuid(&last.payload, "control_id")
                            }),
                        });
                        events.push(EventInput {
                            event_id: Uuid::now_v7(),
                            session_id: session.session_id,
                            occurred_at: now,
                            kind: "session_cancelled".to_owned(),
                            causation_request_id: last.causation_request_id,
                            attempt_id,
                            effect_class: None,
                            payload: terminal_payload("provider cancellation recovered"),
                        });
                        self.telemetry.record_attempt("cancelled");
                    } else {
                        let attempt_id = attempt_id.unwrap_or_else(Uuid::now_v7);
                        events.push(EventInput {
                            event_id: Uuid::now_v7(),
                            session_id: session.session_id,
                            occurred_at: now,
                            kind: "outcome_unknown".to_owned(),
                            causation_request_id: last.causation_request_id,
                            attempt_id: Some(attempt_id),
                            effect_class: None,
                            payload: json!({
                                "attempt_id": attempt_id,
                                "reason": "cancellation recovery could not confirm provider outcome",
                                "reconciliation_options": ["retry", "accept_result", "abandon"]
                            }),
                        });
                        self.telemetry.record_attempt("outcome_unknown");
                    }
                    self.storage.append_events(&events)?;
                    recovered += 1;
                }
                "cancel_confirmed" => {
                    self.storage.append_event(&EventInput {
                        event_id: Uuid::now_v7(),
                        session_id: session.session_id,
                        occurred_at: OffsetDateTime::now_utc(),
                        kind: "session_cancelled".to_owned(),
                        causation_request_id: last.causation_request_id,
                        attempt_id: last.attempt_id,
                        effect_class: None,
                        payload: terminal_payload("confirmed cancellation recovered"),
                    })?;
                    self.telemetry.record_attempt("cancelled");
                    recovered += 1;
                }
                _ => {}
            }
        }
        Ok(recovered)
    }

    fn evict_session(&self, session_id: Uuid) -> Result<(), StorageError> {
        self.session_configs
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .remove(&session_id);
        self.pinned_locks
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .remove(&session_id);
        self.known_sessions
            .lock()
            .map_err(|_| StorageError::StorageUnavailable(None))?
            .remove(&session_id);
        Ok(())
    }

    pub(crate) async fn dispatch(
        self: &Arc<Self>,
        command: ClientCommand,
        context: &ClientContext,
    ) -> DispatchResult {
        let request_id = command.request_id;
        let method = command.command.method();
        info!(method, "protocol command received");
        let _lifecycle_guard = self.lifecycle_gate.read().await;
        let result = if self.shutting_down.load(Ordering::Acquire) {
            Err(DaemonError::InvalidRequest("daemon is shutting down"))
        } else if let Some(session_id) = command.session_id {
            let lock = self.session_lock(session_id).await;
            let _guard = lock.lock().await;
            self.dispatch_session(session_id, command, context).await
        } else {
            match command.command {
                Command::Initialize(params) => Self::handle_initialize(&params),
                Command::StatusGet(_) => self.handle_status().await,
                Command::SessionCreate(params) => {
                    let _guard = self.creation_lock.lock().await;
                    self.handle_create(request_id, &params)
                }
                Command::SessionList(params) => self.handle_list(params),
                _ => Err(DaemonError::InvalidRequest(
                    "session identifier is required",
                )),
            }
        };
        match result {
            Ok(success) => DispatchResult {
                reply: ServerReply::Success {
                    request_id,
                    result: success.result,
                },
                subscription: success.subscription,
            },
            Err(error) => {
                warn!(
                    method,
                    category = error.code_name(),
                    "protocol command failed"
                );
                DispatchResult {
                    reply: ServerReply::Failure {
                        request_id,
                        error: error.into_protocol(),
                    },
                    subscription: None,
                }
            }
        }
    }

    async fn dispatch_session(
        self: &Arc<Self>,
        session_id: Uuid,
        command: ClientCommand,
        context: &ClientContext,
    ) -> Result<CommandSuccess, DaemonError> {
        match command.command {
            Command::SessionGet(_) => self.handle_get(session_id),
            Command::SessionAttach(params) => self.handle_attach(session_id, params),
            Command::SessionPrompt(params) => {
                self.handle_prompt(session_id, command.request_id, params)
                    .await
            }
            Command::SessionPause(_) => {
                self.handle_pause(session_id, command.request_id, context)
                    .await
            }
            Command::SessionResume(_) => {
                self.handle_resume(session_id, command.request_id, context)
                    .await
            }
            Command::SessionRedirect(params) => {
                self.handle_redirect(session_id, command.request_id, context, params)
            }
            Command::SessionCancel(_) => {
                self.handle_cancel(session_id, command.request_id, context)
                    .await
            }
            Command::SessionApprovalResolve(params) => {
                self.handle_approval(session_id, command.request_id, context, params)
                    .await
            }
            Command::SessionReconcile(params) => {
                self.handle_reconcile(session_id, command.request_id, params)
                    .await
            }
            Command::SessionExport(params) => {
                self.handle_export(session_id, command.request_id, &params)
            }
            Command::SessionDelete(params) => {
                self.handle_delete(session_id, command.request_id, context, params)
                    .await
            }
            _ => Err(DaemonError::InvalidRequest(
                "method is not valid for a session",
            )),
        }
    }

    fn handle_initialize(
        params: &workbench_protocol::command::InitializeParams,
    ) -> Result<CommandSuccess, DaemonError> {
        if !params
            .supported_protocols
            .iter()
            .any(|protocol| protocol == workbench_protocol::PROTOCOL_V1)
        {
            return Err(DaemonError::UnsupportedVersion);
        }
        success(InitializeResult {
            selected_protocol: ProtocolVersion::V1,
            max_frame_bytes: 8_388_608,
            max_client_queue_events: 1_024,
            max_client_queue_bytes: 8_388_608,
        })
    }

    async fn handle_status(&self) -> Result<CommandSuccess, DaemonError> {
        let active_sessions = self
            .known_sessions
            .lock()
            .map_err(|_| DaemonError::Internal)?
            .len();
        let mut adapters = Vec::with_capacity(self.provider_catalog.len() + 1);
        for (id, capabilities) in &self.provider_catalog {
            let live_available = if capabilities.authentication == Authentication::Available {
                let adapter = ProviderId::parse(id.clone())
                    .ok()
                    .and_then(|provider| self.providers.adapter(&provider));
                if let Some(adapter) = adapter {
                    matches!(
                        tokio::time::timeout(
                            Duration::from_secs(1),
                            adapter.authentication_status(),
                        )
                        .await,
                        Ok(Ok(workbench_core::ports::AuthenticationStatus::Available))
                    )
                } else {
                    false
                }
            } else {
                false
            };
            adapters.push(AdapterHealth {
                id: id.clone(),
                status: if live_available {
                    AdapterStatus::Available
                } else {
                    AdapterStatus::Unavailable
                },
            });
        }
        if self
            .startup
            .resolved
            .providers
            .iter()
            .any(|(name, provider)| name == "fake" && provider.kind == ProviderType::Fake)
            && !adapters.iter().any(|adapter| adapter.id == "fake")
        {
            adapters.push(AdapterHealth {
                id: "fake".to_owned(),
                status: AdapterStatus::Available,
            });
        }
        adapters.sort_by(|left, right| left.id.cmp(&right.id));
        success(StatusResult {
            daemon_version: DAEMON_VERSION.to_owned(),
            protocol: ProtocolVersion::V1,
            storage_schema_version: 1,
            key_store: KeyStoreStatus::Available,
            migration: MigrationStatus::Ready,
            active_sessions: u64::try_from(active_sessions).map_err(|_| DaemonError::Internal)?,
            adapters,
        })
    }

    fn handle_create(
        &self,
        request_id: Uuid,
        params: &CreateSessionParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = serde_json::to_value(params)?;
        if let Some(outcome) =
            self.storage
                .lookup_command_outcome(None, request_id, "session.create", &parameters)?
        {
            let session_id = uuid_field(&outcome, "session_id")?;
            if !self.storage.is_deleted(session_id)? {
                let stored = self.storage.load_session(session_id)?;
                self.hydrate_stored_session(&stored)?;
            }
            return value_success(outcome);
        }
        let (resolved, snapshot, session_lock) = self
            .startup
            .resolve_session(params.configuration_overrides.as_ref())?;
        let session_id = Uuid::now_v7();
        let lock_hash = session_lock.hash()?;
        let configuration_hash = snapshot.content_hash.clone();
        let command_outcome = serde_json::to_value(CreateSessionResult {
            session_id,
            configuration_hash: configuration_hash.clone(),
            lock_hash: lock_hash.clone(),
            state: ReadyState::Ready,
        })?;
        let outcome = self.storage.create_session(&CreateSession {
            session_id,
            request_id,
            occurred_at: OffsetDateTime::now_utc(),
            request_parameters: parameters,
            command_outcome: command_outcome.clone(),
            configuration_snapshot: serde_json::to_value(&snapshot)?,
            lock_snapshot: serde_json::to_value(&session_lock)?,
            initial_event_payload: json!({
                "configuration_hash": configuration_hash,
                "lock_hash": lock_hash,
            }),
        })?;
        let result = match outcome {
            CommandOutcome::Recorded(value) => {
                self.session_configs
                    .lock()
                    .map_err(|_| DaemonError::Internal)?
                    .insert(session_id, resolved.configuration);
                self.pinned_locks
                    .lock()
                    .map_err(|_| DaemonError::Internal)?
                    .insert(session_id, session_lock);
                self.known_sessions
                    .lock()
                    .map_err(|_| DaemonError::Internal)?
                    .insert(session_id);
                value
            }
            CommandOutcome::Replay(value) => {
                let replayed_session_id = uuid_field(&value, "session_id")?;
                let stored = self.storage.load_session(replayed_session_id)?;
                self.hydrate_stored_session(&stored)?;
                value
            }
        };
        value_success(result)
    }

    fn handle_list(&self, params: ListSessionsParams) -> Result<CommandSuccess, DaemonError> {
        let page = self
            .storage
            .list_session_metadata(params.limit, params.before_session_id)?;
        let sessions = page
            .sessions
            .into_iter()
            .map(|session| {
                Ok(SessionSummary {
                    session_id: session.session_id,
                    state: parse_session_state(&session.state)?,
                    created_at: format_time(session.created_at)?,
                    terminal_at: session.terminal_at.map(format_time).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, DaemonError>>()?;
        success(ListSessionsResult {
            sessions,
            next_before_session_id: page.next_before_session_id,
        })
    }

    fn handle_get(&self, session_id: Uuid) -> Result<CommandSuccess, DaemonError> {
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        success(session_result(session_id, &folded))
    }

    fn handle_attach(
        &self,
        session_id: Uuid,
        params: AttachSessionParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        let replay = history
            .into_iter()
            .filter(|event| event.sequence > params.after_sequence)
            .map(protocol_event)
            .collect::<Result<Vec<_>, _>>()?;
        let subscription = self.subscriptions.subscribe(session_id, replay)?;
        let result = AttachSessionResult {
            session_id,
            state: folded.state,
            replay_after_sequence: params.after_sequence,
            last_sequence: folded.last_sequence,
        };
        Ok(CommandSuccess {
            result: serde_json::to_value(result)?,
            subscription: Some(subscription),
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_prompt(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        params: PromptParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let prompt_content = params.text.clone();
        let parameters = serde_json::to_value(&params)?;
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.prompt", &parameters)?
        {
            return replay_command_outcome(outcome);
        }
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        if folded.state != SessionState::Ready {
            return Err(DaemonError::InvalidTransition);
        }
        let input_id = Uuid::now_v7();
        let input_sequence = folded
            .last_sequence
            .checked_add(1)
            .ok_or(DaemonError::Internal)?;
        let outcome = serde_json::to_value(PromptResult {
            input_id,
            sequence: input_sequence,
        })?;
        let now = OffsetDateTime::now_utc();
        let mut events = vec![command_event(
            session_id,
            request_id,
            now,
            EventKind::InputRecorded,
            json!({"input_id": input_id, "content": params.text}),
            None,
            None,
        )];
        match self.plan_route(session_id, params.explicit_target.as_deref())? {
            RouteDecision::CapabilityUnavailable { selected_rule } => {
                let failure = durable_failure(DaemonError::CapabilityUnavailable);
                let (outcome, _) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.prompt",
                    &parameters,
                    &failure,
                    &events,
                )?;
                self.telemetry.record_route(selected_rule, "failed");
                replay_command_outcome(outcome)
            }
            RouteDecision::Clarification {
                reason,
                selected_rule,
            } => {
                events.push(command_event(
                    session_id,
                    request_id,
                    now,
                    EventKind::ClarificationRequested,
                    json!({
                        "question": "Which configured role should execute this prompt?",
                        "reason": reason
                    }),
                    None,
                    None,
                ));
                let (outcome, _) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.prompt",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                self.telemetry.record_route(selected_rule, "failed");
                self.telemetry.record_route("clarification", "success");
                replay_command_outcome(outcome)
            }
            RouteDecision::Selected { plan, permission } => {
                let selected_rule = selected_rule_name(plan.selected_by);
                events.push(command_event(
                    session_id,
                    request_id,
                    now,
                    EventKind::RoutingPlanned,
                    routing_payload(&plan),
                    None,
                    None,
                ));
                if permission == DefaultToolMode::Denied {
                    let failure = durable_failure(DaemonError::PolicyDenied);
                    let (outcome, _) = self.commit_command_events(
                        session_id,
                        request_id,
                        "session.prompt",
                        &parameters,
                        &failure,
                        &events,
                    )?;
                    self.telemetry.record_route(selected_rule, "denied");
                    return replay_command_outcome(outcome);
                }
                if permission == DefaultToolMode::ApprovalRequired {
                    let approval_id = Uuid::now_v7();
                    events.push(command_event(
                        session_id,
                        request_id,
                        now,
                        EventKind::ApprovalRequested,
                        json!({
                            "approval_id": approval_id,
                            "action": "provider.prompt",
                            "risk": "medium",
                            "scope": ["provider"]
                        }),
                        None,
                        None,
                    ));
                    let (outcome, _) = self.commit_command_events(
                        session_id,
                        request_id,
                        "session.prompt",
                        &parameters,
                        &outcome,
                        &events,
                    )?;
                    self.telemetry.record_route(selected_rule, "success");
                    return replay_command_outcome(outcome);
                }
                let attempt_id = Uuid::now_v7();
                events.extend(dispatch_events(
                    session_id, request_id, now, attempt_id, None,
                ));
                let (outcome, replayed) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.prompt",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                self.telemetry.record_route(selected_rule, "success");
                if !replayed {
                    self.activate_attempt(
                        session_id,
                        request_id,
                        attempt_id,
                        plan.destination.provider,
                        plan.destination.runtime_model,
                        prompt_content,
                    )
                    .await?;
                }
                replay_command_outcome(outcome)
            }
        }
    }

    async fn handle_pause(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = json!({});
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.pause", &parameters)?
        {
            return replay_command_outcome(outcome);
        }
        let state = self.current_session_state(session_id)?;
        if state == SessionState::Paused || state == SessionState::Pausing {
            let outcome = serde_json::to_value(ControlResult {
                control_id: request_id,
                control: Control::Pause,
                state: SessionState::Paused,
            })?;
            return value_success(self.record_command_outcome(
                session_id,
                request_id,
                "session.pause",
                &parameters,
                &outcome,
            )?);
        }
        if state != SessionState::Running {
            return Err(DaemonError::InvalidTransition);
        }
        if self
            .active
            .lock()
            .await
            .get(&session_id)
            .is_some_and(|execution| !execution.is_fake())
        {
            return Err(DaemonError::CapabilityUnavailable);
        }
        let control_id = Uuid::now_v7();
        let outcome = serde_json::to_value(ControlResult {
            control_id,
            control: Control::Pause,
            state: SessionState::Paused,
        })?;
        let now = OffsetDateTime::now_utc();
        let events = [
            command_event(
                session_id,
                request_id,
                now,
                EventKind::PauseRequested,
                control_payload(control_id, context, None),
                None,
                None,
            ),
            command_event(
                session_id,
                request_id,
                now,
                EventKind::SessionPaused,
                control_payload(control_id, context, None),
                None,
                None,
            ),
        ];
        let (outcome, _) = self.commit_command_events(
            session_id,
            request_id,
            "session.pause",
            &parameters,
            &outcome,
            &events,
        )?;
        replay_command_outcome(outcome)
    }

    async fn handle_resume(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = json!({});
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.resume", &parameters)?
        {
            return value_success(outcome);
        }
        let state = self.current_session_state(session_id)?;
        if state == SessionState::Running {
            let outcome = serde_json::to_value(ControlResult {
                control_id: request_id,
                control: Control::Resume,
                state: SessionState::Running,
            })?;
            return value_success(self.record_command_outcome(
                session_id,
                request_id,
                "session.resume",
                &parameters,
                &outcome,
            )?);
        }
        if state != SessionState::Paused {
            return Err(DaemonError::InvalidTransition);
        }
        let control_id = Uuid::now_v7();
        let outcome = serde_json::to_value(ControlResult {
            control_id,
            control: Control::Resume,
            state: SessionState::Running,
        })?;
        let (outcome, replayed) = self.commit_command_event(
            session_id,
            request_id,
            "session.resume",
            &parameters,
            &outcome,
            EventKind::SessionResumed,
            control_payload(control_id, context, None),
            None,
            None,
        )?;
        if replayed {
            return value_success(outcome);
        }
        if self
            .active
            .lock()
            .await
            .get(&session_id)
            .is_some_and(|execution| execution.is_fake())
        {
            self.schedule_fake_completion(session_id);
        }
        value_success(outcome)
    }

    fn handle_redirect(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
        params: RedirectParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = serde_json::to_value(&params)?;
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.redirect", &parameters)?
        {
            return value_success(outcome);
        }
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        if !matches!(
            folded.state,
            SessionState::Paused | SessionState::AwaitingClarification
        ) {
            return Err(DaemonError::InvalidTransition);
        }
        let control_id = Uuid::now_v7();
        let next_state = if folded.state == SessionState::AwaitingClarification {
            SessionState::Ready
        } else {
            SessionState::Paused
        };
        let outcome = serde_json::to_value(ControlResult {
            control_id,
            control: Control::Redirect,
            state: next_state,
        })?;
        let (outcome, replayed) = self.commit_command_event(
            session_id,
            request_id,
            "session.redirect",
            &parameters,
            &outcome,
            EventKind::SessionRedirected,
            control_payload(control_id, context, Some(params.instruction)),
            None,
            None,
        )?;
        if replayed {
            return value_success(outcome);
        }
        value_success(outcome)
    }

    async fn handle_cancel(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = json!({});
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.cancel", &parameters)?
        {
            let history = self.history(session_id)?;
            if fold_session(&history)?.state == SessionState::CancelRequested {
                let control_id = uuid_field(&outcome, "control_id")?;
                self.schedule_cancel_resolution(
                    session_id,
                    request_id,
                    control_id,
                    latest_started_attempt(&history),
                );
            }
            return replay_command_outcome(outcome);
        }
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        if matches!(
            folded.state,
            SessionState::Completed
                | SessionState::Failed
                | SessionState::Cancelled
                | SessionState::Abandoned
                | SessionState::Deleting
        ) {
            return Err(DaemonError::InvalidTransition);
        }
        let control_id = Uuid::now_v7();
        let attempt_id = self
            .active
            .lock()
            .await
            .get(&session_id)
            .map(|active| active.attempt_id)
            .or_else(|| latest_started_attempt(&history));
        let terminal_without_attempt = attempt_id.is_none();
        let outcome = serde_json::to_value(ControlResult {
            control_id,
            control: Control::Cancel,
            state: if terminal_without_attempt {
                SessionState::Cancelled
            } else {
                SessionState::CancelRequested
            },
        })?;
        let now = OffsetDateTime::now_utc();
        let mut events = vec![command_event(
            session_id,
            request_id,
            now,
            EventKind::CancelRequested,
            control_payload(control_id, context, None),
            None,
            None,
        )];
        if terminal_without_attempt {
            events.push(command_event(
                session_id,
                request_id,
                now,
                EventKind::CancelConfirmed,
                json!({"control_id": control_id}),
                None,
                None,
            ));
            events.push(command_event(
                session_id,
                request_id,
                now,
                EventKind::SessionCancelled,
                terminal_payload("cancelled before any external attempt started"),
                None,
                None,
            ));
        }
        let (outcome, replayed) = self.commit_command_events(
            session_id,
            request_id,
            "session.cancel",
            &parameters,
            &outcome,
            &events,
        )?;
        if replayed {
            return replay_command_outcome(outcome);
        }
        if !terminal_without_attempt {
            self.schedule_cancel_resolution(session_id, request_id, control_id, attempt_id);
        }
        replay_command_outcome(outcome)
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_approval(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
        params: ApprovalParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = serde_json::to_value(&params)?;
        if let Some(outcome) = self.cached_outcome(
            session_id,
            request_id,
            "session.approval.resolve",
            &parameters,
        )? {
            if params.decision == ApprovalDecision::Grant {
                let history = self.history(session_id)?;
                if fold_session(&history)?.state == SessionState::Running
                    && let Some(attempt_id) = latest_started_attempt(&history)
                    && !self.active.lock().await.contains_key(&session_id)
                {
                    let causation = history
                        .iter()
                        .rev()
                        .find(|event| event.kind == "dispatch_started")
                        .and_then(|event| event.causation_request_id)
                        .unwrap_or(request_id);
                    let context = provider_execution_context(&history, causation)?;
                    self.activate_attempt(
                        session_id,
                        causation,
                        attempt_id,
                        context.provider,
                        context.runtime_model,
                        context.prompt,
                    )
                    .await?;
                }
            }
            return replay_command_outcome(outcome);
        }
        let folded = fold_session(&self.history(session_id)?)?;
        if let Some(recorded) = folded.approval_decisions.get(&params.approval_id) {
            if *recorded != params.decision {
                return Err(DaemonError::InvalidTransition);
            }
            let outcome = serde_json::to_value(ApprovalResult {
                approval_id: params.approval_id,
                decision: params.decision,
                state: match recorded {
                    ApprovalDecision::Grant => SessionState::Running,
                    ApprovalDecision::Deny => SessionState::Paused,
                },
            })?;
            return value_success(self.record_command_outcome(
                session_id,
                request_id,
                "session.approval.resolve",
                &parameters,
                &outcome,
            )?);
        }
        if folded.state != SessionState::AwaitingApproval
            || folded.pending_approval_id != Some(params.approval_id)
        {
            return Err(DaemonError::InvalidTransition);
        }
        let outcome_state = match params.decision {
            ApprovalDecision::Grant => SessionState::Running,
            ApprovalDecision::Deny => SessionState::Paused,
        };
        let outcome = serde_json::to_value(ApprovalResult {
            approval_id: params.approval_id,
            decision: params.decision,
            state: outcome_state,
        })?;
        let now = OffsetDateTime::now_utc();
        let mut events = vec![command_event(
            session_id,
            request_id,
            now,
            EventKind::ApprovalRecorded,
            json!({
                "approval_id": params.approval_id,
                "actor": context.actor(),
                "decision": params.decision
            }),
            None,
            None,
        )];
        match params.decision {
            ApprovalDecision::Grant => {
                let causation = folded.pending_request_id.unwrap_or(request_id);
                let attempt_id = Uuid::now_v7();
                events.extend(dispatch_events(
                    session_id, causation, now, attempt_id, None,
                ));
                let (outcome, replayed) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.approval.resolve",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                if !replayed {
                    let history = self.history(session_id)?;
                    let context = provider_execution_context(&history, causation)?;
                    self.activate_attempt(
                        session_id,
                        causation,
                        attempt_id,
                        context.provider,
                        context.runtime_model,
                        context.prompt,
                    )
                    .await?;
                }
                replay_command_outcome(outcome)
            }
            ApprovalDecision::Deny => {
                events.push(command_event(
                    session_id,
                    request_id,
                    now,
                    EventKind::SessionPaused,
                    control_payload(Uuid::now_v7(), context, None),
                    None,
                    None,
                ));
                let (outcome, _) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.approval.resolve",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                replay_command_outcome(outcome)
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_reconcile(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        params: ReconciliationParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = serde_json::to_value(&params)?;
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.reconcile", &parameters)?
        {
            return replay_command_outcome(outcome);
        }
        let folded = fold_session(&self.history(session_id)?)?;
        if folded.state != SessionState::OutcomeUnknown
            || folded.uncertain_attempt_id != Some(params.attempt_id)
        {
            return Err(DaemonError::InvalidTransition);
        }
        let replacement_attempt_id =
            (params.resolution == ReconciliationResolution::Retry).then(Uuid::now_v7);
        let outcome_state = match params.resolution {
            ReconciliationResolution::Retry => SessionState::Running,
            ReconciliationResolution::AcceptResult => SessionState::Completed,
            ReconciliationResolution::Abandon => SessionState::Abandoned,
        };
        let outcome = serde_json::to_value(ReconciliationResult {
            attempt_id: params.attempt_id,
            resolution: params.resolution,
            replacement_attempt_id,
            state: outcome_state,
        })?;
        let now = OffsetDateTime::now_utc();
        let mut events = vec![command_event(
            session_id,
            request_id,
            now,
            EventKind::OutcomeReconciled,
            json!({
                "attempt_id": params.attempt_id,
                "resolution": params.resolution,
                "replacement_attempt_id": replacement_attempt_id,
                "evidence_recorded": params.evidence.is_some()
            }),
            Some(params.attempt_id),
            None,
        )];
        match params.resolution {
            ReconciliationResolution::Retry => {
                let replacement_attempt_id = replacement_attempt_id.ok_or(DaemonError::Internal)?;
                events.extend(dispatch_events(
                    session_id,
                    request_id,
                    now,
                    replacement_attempt_id,
                    Some(params.attempt_id),
                ));
                let (outcome, replayed) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.reconcile",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                if !replayed {
                    let history = self.history(session_id)?;
                    let context = provider_execution_context(&history, request_id)?;
                    self.activate_attempt(
                        session_id,
                        request_id,
                        replacement_attempt_id,
                        context.provider,
                        context.runtime_model,
                        context.prompt,
                    )
                    .await?;
                }
                replay_command_outcome(outcome)
            }
            ReconciliationResolution::AcceptResult => {
                events.push(command_event(
                    session_id,
                    request_id,
                    now,
                    EventKind::SessionCompleted,
                    terminal_payload("result accepted by human reconciliation"),
                    Some(params.attempt_id),
                    None,
                ));
                let (outcome, _) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.reconcile",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                replay_command_outcome(outcome)
            }
            ReconciliationResolution::Abandon => {
                events.push(command_event(
                    session_id,
                    request_id,
                    now,
                    EventKind::SessionAbandoned,
                    terminal_payload("execution abandoned by human reconciliation"),
                    Some(params.attempt_id),
                    None,
                ));
                let (outcome, _) = self.commit_command_events(
                    session_id,
                    request_id,
                    "session.reconcile",
                    &parameters,
                    &outcome,
                    &events,
                )?;
                replay_command_outcome(outcome)
            }
        }
    }

    fn handle_export(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        params: &ExportParams,
    ) -> Result<CommandSuccess, DaemonError> {
        let parameters = serde_json::to_value(params)?;
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.export", &parameters)?
        {
            return value_success(outcome);
        }
        fold_session(&self.history(session_id)?)?;
        let export_id = Uuid::now_v7();
        let recipient_fingerprints = recipient_fingerprints(&params.age_recipients)?;
        let outcome = serde_json::to_value(ExportResult {
            export_id,
            format: ExportFormat::AgeV1,
            recipient_fingerprints: recipient_fingerprints.clone(),
        })?;
        let committed = self.storage.execute_export(&ExportCommand {
            session_id,
            request_id,
            export_id,
            occurred_at: OffsetDateTime::now_utc(),
            parameters: parameters.clone(),
            output_path: PathBuf::from(&params.output_path),
            age_recipients: params.age_recipients.clone(),
            outcome,
            event_payload: json!({
                "export_id": export_id,
                "format": "age-v1",
                "recipient_fingerprints": recipient_fingerprints
            }),
        })?;
        let stored_outcome = match committed {
            CommandEventOutcome::Recorded { event, outcome } => {
                self.subscriptions.publish(&protocol_event(event)?);
                outcome
            }
            CommandEventOutcome::Replay(outcome) => outcome,
        };
        value_success(stored_outcome)
    }

    async fn handle_delete(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        context: &ClientContext,
        params: DeleteParams,
    ) -> Result<CommandSuccess, DaemonError> {
        if params.confirm_session_id != session_id {
            return Err(DaemonError::InvalidRequest(
                "delete confirmation does not match session",
            ));
        }
        let parameters = serde_json::to_value(&params)?;
        if let Some(outcome) =
            self.cached_outcome(session_id, request_id, "session.delete", &parameters)?
        {
            return success(DeleteResult {
                deletion_id: uuid_field(&outcome, "deletion_id")?,
                state: DeleteState::Deleted,
            });
        }
        let history = self.history(session_id)?;
        let folded = fold_session(&history)?;
        if !matches!(
            folded.state,
            SessionState::Completed
                | SessionState::Failed
                | SessionState::Cancelled
                | SessionState::Abandoned
        ) {
            return Err(DaemonError::InvalidTransition);
        }
        let deletion_id = Uuid::now_v7();
        let next_sequence = folded
            .last_sequence
            .checked_add(2)
            .ok_or(DaemonError::Internal)?;
        self.storage.ensure_session_deletable(session_id)?;
        drop(history);
        let summary = self.request_deletion(
            session_id,
            deletion_id,
            request_id,
            OffsetDateTime::now_utc(),
            &context.actor(),
        )?;
        self.active.lock().await.remove(&session_id);
        self.evict_session(session_id)?;
        self.subscriptions.purge_session(session_id)?;
        self.subscriptions.publish(&SessionEvent {
            protocol: workbench_protocol::PROTOCOL_V1.to_owned(),
            event_id: Uuid::now_v7(),
            session_id,
            sequence: next_sequence,
            causation_request_id: Some(request_id),
            kind: EventKind::SessionDeleted,
            occurred_at: format_time(OffsetDateTime::now_utc())?,
            data: json!({
                "deletion_id": summary.deletion_id,
                "key_destroyed": summary.key_destroyed
            }),
        });
        success(DeleteResult {
            deletion_id: summary.deletion_id,
            state: DeleteState::Deleted,
        })
    }

    fn plan_route(
        &self,
        session_id: Uuid,
        explicit_target: Option<&str>,
    ) -> Result<RouteDecision, DaemonError> {
        let config = self.session_configuration(session_id)?;
        let mut available = self.provider_catalog.clone();
        available.extend(fake_provider_capabilities(&config));
        let selected_rule = if explicit_target.is_some() {
            "explicit"
        } else {
            "coordinator"
        };
        let role_name = explicit_target.unwrap_or(&config.routing.default_role);
        let resolved = match resolve_role(&config, role_name, &available) {
            Ok(resolved) => resolved,
            Err(workbench_config::ConfigError::CapabilityUnavailable { .. }) => {
                return Ok(RouteDecision::CapabilityUnavailable { selected_rule });
            }
            Err(workbench_config::ConfigError::Invalid { path, .. }) if path == "role" => {
                return Ok(RouteDecision::Clarification {
                    reason: format!("configured role {role_name} does not exist"),
                    selected_rule,
                });
            }
            Err(_) => {
                return Err(DaemonError::InvalidRequest(
                    "routing configuration is invalid",
                ));
            }
        };
        let candidate =
            route_candidate(&config, &resolved, effective_provider_permission(&config))?;
        let inputs = if explicit_target.is_some() {
            RoutingInputs {
                explicit: Some(candidate),
                ..RoutingInputs::default()
            }
        } else {
            RoutingInputs {
                coordinator: Some(candidate),
                ..RoutingInputs::default()
            }
        };
        match OrderedRouter::new(config.routing.confidence_threshold)
            .map_err(|_| DaemonError::InvalidRequest("routing threshold is invalid"))?
            .resolve(inputs)
        {
            RoutingOutcome::Selected(plan) => Ok(RouteDecision::Selected {
                permission: permission_from_scope(plan.context.permission),
                plan,
            }),
            RoutingOutcome::NeedsClarification { reason, .. } => Ok(RouteDecision::Clarification {
                reason: reason.to_owned(),
                selected_rule,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn activate_attempt(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        attempt_id: Uuid,
        provider: ProviderId,
        runtime_model: String,
        prompt: String,
    ) -> Result<(), DaemonError> {
        let configuration = self.session_configuration(session_id)?;
        let kind = configuration
            .providers
            .get(provider.as_str())
            .map(|provider| provider.kind)
            .ok_or(DaemonError::CapabilityUnavailable)?;
        if kind == ProviderType::Fake {
            self.activate_fake_attempt(session_id, request_id, attempt_id)
                .await;
            return Ok(());
        }

        let Some(adapter) = self.providers.adapter(&provider) else {
            self.record_provider_failure(
                session_id,
                request_id,
                attempt_id,
                &ProviderFailure {
                    category: workbench_core::FailureCategory::CapabilityUnavailable,
                    user_safe_message: "configured provider adapter is unavailable".to_owned(),
                    definite: true,
                },
            )?;
            return Ok(());
        };
        let handle = match adapter.start_session().await {
            Ok(handle) => handle,
            Err(failure) => {
                self.record_provider_failure(session_id, request_id, attempt_id, &failure)?;
                return Ok(());
            }
        };
        let prompt = ProviderPrompt {
            session_id: SessionId::from_uuid(session_id),
            attempt_id: AttemptId::from_uuid(attempt_id),
            runtime_model,
            content: NonEmptyText::parse(prompt)
                .map_err(|_| DaemonError::InvalidRequest("prompt text must not be empty"))?,
        };
        if kind == ProviderType::SubscriptionCli {
            self.active.lock().await.insert(
                session_id,
                Arc::new(ActiveExecution::provider(
                    attempt_id,
                    request_id,
                    Arc::clone(&adapter),
                    handle.clone(),
                )),
            );
            self.schedule_provider_activation(session_id, attempt_id, adapter, handle, prompt);
            return Ok(());
        }
        let stream = match adapter.prompt_stream(&handle, prompt).await {
            Ok(stream) => stream,
            Err(failure) => {
                self.record_provider_failure(session_id, request_id, attempt_id, &failure)?;
                return Ok(());
            }
        };
        self.active.lock().await.insert(
            session_id,
            Arc::new(ActiveExecution::provider(
                attempt_id, request_id, adapter, handle,
            )),
        );
        let application = Arc::clone(self);
        drop(tokio::spawn(async move {
            if let Err(error) = application
                .consume_provider_stream(session_id, attempt_id, stream)
                .await
            {
                application.telemetry.record_attempt("failed");
                warn!(
                    category = error.code_name(),
                    "provider stream processing failed"
                );
            }
        }));
        Ok(())
    }

    fn schedule_provider_activation(
        self: &Arc<Self>,
        session_id: Uuid,
        attempt_id: Uuid,
        adapter: Arc<dyn ProviderAdapter>,
        handle: ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) {
        let application = Arc::clone(self);
        drop(tokio::spawn(async move {
            match adapter.prompt_stream(&handle, prompt).await {
                Ok(stream) => {
                    if let Err(error) = application
                        .consume_provider_stream(session_id, attempt_id, stream)
                        .await
                    {
                        application.telemetry.record_attempt("failed");
                        warn!(
                            category = error.code_name(),
                            "provider stream processing failed"
                        );
                    }
                }
                Err(failure) => {
                    if let Err(error) = application
                        .finish_provider_activation_failure(session_id, attempt_id, &failure)
                        .await
                    {
                        application.telemetry.record_attempt("failed");
                        warn!(
                            category = error.code_name(),
                            "provider activation failure could not be recorded"
                        );
                    }
                }
            }
        }));
    }

    async fn finish_provider_activation_failure(
        &self,
        session_id: Uuid,
        attempt_id: Uuid,
        failure: &ProviderFailure,
    ) -> Result<(), DaemonError> {
        let _lifecycle_guard = self.lifecycle_gate.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        let Some(active) = self.active_for_attempt(session_id, attempt_id).await else {
            return Ok(());
        };
        if self.current_session_state(session_id)? == SessionState::CancelRequested {
            return Ok(());
        }
        self.record_provider_failure(session_id, active.request_id, attempt_id, failure)?;
        self.remove_active_attempt(session_id, attempt_id).await;
        Ok(())
    }

    async fn activate_fake_attempt(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        attempt_id: Uuid,
    ) {
        self.active.lock().await.insert(
            session_id,
            Arc::new(ActiveExecution::fake(attempt_id, request_id)),
        );
        self.schedule_fake_completion(session_id);
    }

    async fn consume_provider_stream(
        self: &Arc<Self>,
        session_id: Uuid,
        attempt_id: Uuid,
        mut stream: workbench_core::ports::ProviderStream,
    ) -> Result<(), DaemonError> {
        while let Some(item) = stream.next().await {
            let _lifecycle_guard = self.lifecycle_gate.read().await;
            if self.shutting_down.load(Ordering::Acquire) {
                return Ok(());
            }
            let lock = self.session_lock(session_id).await;
            let _guard = lock.lock().await;
            let Some(active) = self.active_for_attempt(session_id, attempt_id).await else {
                return Ok(());
            };
            match item {
                Ok(output) => {
                    if self
                        .record_provider_output(session_id, &active, output)?
                        .is_terminal()
                    {
                        self.remove_active_attempt(session_id, attempt_id).await;
                        return Ok(());
                    }
                }
                Err(failure) => {
                    if self.current_session_state(session_id)? == SessionState::CancelRequested {
                        return Ok(());
                    }
                    self.record_provider_failure(
                        session_id,
                        active.request_id,
                        attempt_id,
                        &failure,
                    )?;
                    self.remove_active_attempt(session_id, attempt_id).await;
                    return Ok(());
                }
            }
        }

        let _lifecycle_guard = self.lifecycle_gate.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        let Some(active) = self.active_for_attempt(session_id, attempt_id).await else {
            return Ok(());
        };
        let state = self.current_session_state(session_id)?;
        if state == SessionState::Running {
            self.record_provider_failure(
                session_id,
                active.request_id,
                attempt_id,
                &ProviderFailure {
                    category: workbench_core::FailureCategory::OutcomeUnknown,
                    user_safe_message: "provider stream ended without a definite terminal result"
                        .to_owned(),
                    definite: false,
                },
            )?;
            self.remove_active_attempt(session_id, attempt_id).await;
        }
        Ok(())
    }

    fn record_provider_output(
        &self,
        session_id: Uuid,
        active: &ActiveExecution,
        output: ProviderOutput,
    ) -> Result<ProviderOutputState, DaemonError> {
        match output {
            ProviderOutput::Acknowledged {
                provider_request_id,
            } => {
                self.append(
                    session_id,
                    Some(active.request_id),
                    EventKind::DispatchAcknowledged,
                    json!({
                        "attempt_id": active.attempt_id,
                        "provider_request_id": provider_request_id
                    }),
                    Some(active.attempt_id),
                    Some("paid-inference"),
                )?;
                Ok(ProviderOutputState::Streaming)
            }
            ProviderOutput::Content {
                event_type,
                content,
            } => {
                self.append(
                    session_id,
                    Some(active.request_id),
                    EventKind::ProviderEvent,
                    json!({
                        "attempt_id": active.attempt_id,
                        "event_type": event_type,
                        "content": content.as_str()
                    }),
                    Some(active.attempt_id),
                    Some("paid-inference"),
                )?;
                Ok(ProviderOutputState::Streaming)
            }
            ProviderOutput::Tool {
                event_type,
                content,
            } => {
                self.append(
                    session_id,
                    Some(active.request_id),
                    EventKind::ToolEvent,
                    json!({
                        "attempt_id": active.attempt_id,
                        "event_type": event_type,
                        "content": content.as_str()
                    }),
                    Some(active.attempt_id),
                    Some("paid-inference"),
                )?;
                Ok(ProviderOutputState::Streaming)
            }
            ProviderOutput::Completed { summary } => {
                if self.current_session_state(session_id)? == SessionState::CancelRequested {
                    return Ok(ProviderOutputState::CancellationPending);
                }
                self.append(
                    session_id,
                    Some(active.request_id),
                    EventKind::SessionCompleted,
                    terminal_payload(&summary),
                    Some(active.attempt_id),
                    Some("paid-inference"),
                )?;
                self.telemetry.record_attempt("success");
                Ok(ProviderOutputState::Terminal)
            }
        }
    }

    fn record_provider_failure(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        attempt_id: Uuid,
        failure: &ProviderFailure,
    ) -> Result<(), DaemonError> {
        let cancelling = self.current_session_state(session_id)? == SessionState::CancelRequested;
        if failure.definite && !cancelling {
            self.append(
                session_id,
                Some(request_id),
                EventKind::SessionFailed,
                terminal_payload(&failure.user_safe_message),
                Some(attempt_id),
                Some("paid-inference"),
            )?;
            self.telemetry.record_attempt("failed");
        } else {
            self.append(
                session_id,
                Some(request_id),
                EventKind::OutcomeUnknown,
                json!({
                    "attempt_id": attempt_id,
                    "reason": failure.user_safe_message,
                    "reconciliation_options": ["retry", "accept_result", "abandon"]
                }),
                Some(attempt_id),
                Some("paid-inference"),
            )?;
            self.telemetry.record_attempt("outcome_unknown");
        }
        Ok(())
    }

    async fn active_for_attempt(
        &self,
        session_id: Uuid,
        attempt_id: Uuid,
    ) -> Option<Arc<ActiveExecution>> {
        self.active
            .lock()
            .await
            .get(&session_id)
            .filter(|execution| execution.attempt_id == attempt_id)
            .cloned()
    }

    async fn remove_active_attempt(&self, session_id: Uuid, attempt_id: Uuid) {
        let mut active = self.active.lock().await;
        if active
            .get(&session_id)
            .is_some_and(|execution| execution.attempt_id == attempt_id)
        {
            active.remove(&session_id);
        }
    }

    fn schedule_fake_completion(self: &Arc<Self>, session_id: Uuid) {
        let application = Arc::clone(self);
        let delay = self.fake.response_delay;
        drop(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(error) = application.finish_fake(session_id).await {
                application.telemetry.record_attempt("failed");
                warn!(category = error.code_name(), "fake execution failed");
            }
        }));
    }

    async fn finish_fake(self: &Arc<Self>, session_id: Uuid) -> Result<(), DaemonError> {
        let _lifecycle_guard = self.lifecycle_gate.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        let Some(active) = self.active.lock().await.get(&session_id).cloned() else {
            return Ok(());
        };
        if !active.is_fake() {
            return Ok(());
        }
        let folded = fold_session(&self.history(session_id)?)?;
        if folded.state != SessionState::Running {
            return Ok(());
        }
        self.append(
            session_id,
            Some(active.request_id),
            EventKind::ProviderEvent,
            json!({
                "attempt_id": active.attempt_id,
                "event_type": "message",
                "content": "deterministic fake provider response"
            }),
            Some(active.attempt_id),
            Some("paid-inference"),
        )?;
        self.append(
            session_id,
            Some(active.request_id),
            EventKind::SessionCompleted,
            terminal_payload("deterministic fake provider completed"),
            Some(active.attempt_id),
            Some("paid-inference"),
        )?;
        self.active.lock().await.remove(&session_id);
        Ok(())
    }

    fn schedule_cancel_resolution(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
        control_id: Uuid,
        attempt_id: Option<Uuid>,
    ) {
        let application = Arc::clone(self);
        drop(tokio::spawn(async move {
            let active = application.active.lock().await.get(&session_id).cloned();
            let (confirmed, summary) = match active {
                Some(active)
                    if attempt_id.is_some_and(|attempt_id| attempt_id == active.attempt_id) =>
                {
                    if active.cancel_started.swap(true, Ordering::AcqRel) {
                        return;
                    }
                    match &active.backend {
                        ActiveBackend::Fake => {
                            let behavior = application.fake;
                            if !behavior.confirms_cancellation {
                                tokio::time::sleep(behavior.cancellation_deadline).await;
                            }
                            (
                                behavior.confirms_cancellation,
                                "fake provider confirmed cancellation",
                            )
                        }
                        ActiveBackend::Provider { adapter, handle } => {
                            let confirmed = matches!(
                                adapter
                                    .cancel(handle, AttemptId::from_uuid(active.attempt_id),)
                                    .await,
                                Ok(CancellationStatus::Confirmed)
                            );
                            (confirmed, "provider confirmed cancellation")
                        }
                    }
                }
                _ => (false, "provider cancellation could not be confirmed"),
            };
            let _lifecycle_guard = application.lifecycle_gate.read().await;
            if application.shutting_down.load(Ordering::Acquire) {
                return;
            }
            let lock = application.session_lock(session_id).await;
            let _guard = lock.lock().await;
            let result = application
                .resolve_cancel(
                    session_id, request_id, control_id, attempt_id, confirmed, summary,
                )
                .await;
            if let Err(error) = result {
                application.telemetry.record_attempt("failed");
                warn!(
                    category = error.code_name(),
                    "cancellation resolution failed"
                );
            }
        }));
    }

    async fn resolve_cancel(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        control_id: Uuid,
        attempt_id: Option<Uuid>,
        confirmed: bool,
        confirmed_summary: &str,
    ) -> Result<(), DaemonError> {
        let folded = fold_session(&self.history(session_id)?)?;
        if folded.state != SessionState::CancelRequested {
            return Ok(());
        }
        let occurred_at = OffsetDateTime::now_utc();
        let events = if confirmed {
            vec![
                command_event(
                    session_id,
                    request_id,
                    occurred_at,
                    EventKind::CancelConfirmed,
                    json!({"control_id": control_id}),
                    attempt_id,
                    None,
                ),
                command_event(
                    session_id,
                    request_id,
                    occurred_at,
                    EventKind::SessionCancelled,
                    terminal_payload(confirmed_summary),
                    attempt_id,
                    None,
                ),
            ]
        } else {
            let attempt_id = attempt_id.unwrap_or_else(Uuid::now_v7);
            vec![command_event(
                session_id,
                request_id,
                occurred_at,
                EventKind::OutcomeUnknown,
                json!({
                    "attempt_id": attempt_id,
                    "reason": "cancellation confirmation deadline expired",
                    "reconciliation_options": ["retry", "accept_result", "abandon"]
                }),
                Some(attempt_id),
                None,
            )]
        };
        let persisted = self.storage.append_events(&events)?;
        for event in persisted {
            let event = protocol_event(event)?;
            self.subscriptions.publish(&event);
            self.record_event_telemetry(event.kind);
        }
        self.active.lock().await.remove(&session_id);
        if !confirmed {
            self.telemetry.record_attempt("timeout");
        }
        Ok(())
    }

    /// Persists uncertainty for every active external attempt before shutdown.
    ///
    /// # Errors
    ///
    /// Returns a redacted storage error when the shutdown facts cannot be
    /// durably appended.
    pub async fn prepare_shutdown(&self) -> Result<(), StorageError> {
        let _lifecycle_guard = self.lifecycle_gate.write().await;
        self.shutting_down.store(true, Ordering::Release);
        let active = {
            let mut active = self.active.lock().await;
            std::mem::take(&mut *active)
        };
        for (session_id, execution) in active {
            self.storage.append_event(&EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at: OffsetDateTime::now_utc(),
                kind: "outcome_unknown".to_owned(),
                causation_request_id: None,
                attempt_id: Some(execution.attempt_id),
                effect_class: Some("paid-inference".to_owned()),
                payload: json!({
                    "attempt_id": execution.attempt_id,
                    "reason": "daemon_shutdown",
                    "reconciliation_options": ["retry", "accept_result", "abandon"]
                }),
            })?;
            self.telemetry.record_attempt("outcome_unknown");
        }
        Ok(())
    }

    async fn session_lock(&self, session_id: Uuid) -> Arc<AsyncMutex<()>> {
        let mut locks = self.session_locks.lock().await;
        Arc::clone(
            locks
                .entry(session_id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }

    fn history(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<workbench_storage::PersistedEvent>, DaemonError> {
        #[cfg(test)]
        self.history_replays.fetch_add(1, Ordering::Relaxed);
        let events = self.storage.replay(session_id, 0)?;
        self.known_sessions
            .lock()
            .map_err(|_| DaemonError::Internal)?
            .insert(session_id);
        Ok(events)
    }

    fn session_configuration(
        &self,
        session_id: Uuid,
    ) -> Result<WorkbenchConfiguration, DaemonError> {
        if let Some(configuration) = self
            .session_configs
            .lock()
            .map_err(|_| DaemonError::Internal)?
            .get(&session_id)
            .cloned()
        {
            return Ok(configuration);
        }
        let stored = self.storage.load_session(session_id)?;
        self.hydrate_stored_session(&stored)?;
        self.session_configs
            .lock()
            .map_err(|_| DaemonError::Internal)?
            .get(&session_id)
            .cloned()
            .ok_or(DaemonError::StorageUnavailable)
    }

    #[allow(clippy::too_many_arguments)]
    fn append(
        &self,
        session_id: Uuid,
        request_id: Option<Uuid>,
        kind: EventKind,
        payload: Value,
        attempt_id: Option<Uuid>,
        effect_class: Option<&str>,
    ) -> Result<SessionEvent, DaemonError> {
        let persisted = self.storage.append_event(&EventInput {
            event_id: Uuid::now_v7(),
            session_id,
            occurred_at: OffsetDateTime::now_utc(),
            kind: event_kind_name(kind).to_owned(),
            causation_request_id: request_id,
            attempt_id,
            effect_class: effect_class.map(str::to_owned),
            payload,
        })?;
        let event = protocol_event(persisted)?;
        self.subscriptions.publish(&event);
        self.record_event_telemetry(kind);
        Ok(event)
    }

    fn cached_outcome(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
    ) -> Result<Option<Value>, DaemonError> {
        self.storage
            .lookup_command_outcome(Some(session_id), request_id, method, parameters)
            .map_err(DaemonError::from)
    }

    fn record_command_outcome(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
    ) -> Result<Value, DaemonError> {
        match self
            .storage
            .record_command_outcome(session_id, request_id, method, parameters, outcome)?
        {
            CommandOutcome::Recorded(value) | CommandOutcome::Replay(value) => Ok(value),
        }
    }

    fn current_session_state(&self, session_id: Uuid) -> Result<SessionState, DaemonError> {
        parse_session_state(&self.storage.load_session_state(session_id)?)
    }

    fn request_deletion(
        &self,
        session_id: Uuid,
        deletion_id: Uuid,
        request_id: Uuid,
        occurred_at: OffsetDateTime,
        actor: &str,
    ) -> Result<DeletionSummary, StorageError> {
        #[cfg(test)]
        if self
            .fail_next_deletion_request
            .swap(false, Ordering::AcqRel)
        {
            return Err(StorageError::StorageUnavailable(None));
        }
        self.storage
            .request_deletion(session_id, deletion_id, request_id, occurred_at, actor)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_command_events(
        &self,
        _session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        events: &[EventInput],
    ) -> Result<(Value, bool), DaemonError> {
        #[cfg(test)]
        if self.fail_next_command_commit.swap(false, Ordering::AcqRel) {
            return Err(DaemonError::StorageUnavailable);
        }
        let committed = self
            .storage
            .commit_command_events(request_id, method, parameters, outcome, events)?;
        let (stored_outcome, replayed) = match committed {
            CommandEventsOutcome::Recorded { events, outcome } => {
                for persisted in events {
                    let event = protocol_event(persisted)?;
                    self.subscriptions.publish(&event);
                    self.record_event_telemetry(event.kind);
                }
                (outcome, false)
            }
            CommandEventsOutcome::Replay(value) => (value, true),
        };
        Ok((stored_outcome, replayed))
    }

    fn record_event_telemetry(&self, kind: EventKind) {
        match kind {
            EventKind::SessionCompleted => self.telemetry.record_attempt("success"),
            EventKind::SessionFailed => self.telemetry.record_attempt("failed"),
            EventKind::SessionCancelled => self.telemetry.record_attempt("cancelled"),
            EventKind::SessionAbandoned => self.telemetry.record_attempt("abandoned"),
            EventKind::OutcomeUnknown => self.telemetry.record_attempt("outcome_unknown"),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_command_event(
        &self,
        session_id: Uuid,
        request_id: Uuid,
        method: &str,
        parameters: &Value,
        outcome: &Value,
        kind: EventKind,
        payload: Value,
        attempt_id: Option<Uuid>,
        effect_class: Option<&str>,
    ) -> Result<(Value, bool), DaemonError> {
        let committed = self.storage.commit_command_event(
            request_id,
            method,
            parameters,
            outcome,
            &EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at: OffsetDateTime::now_utc(),
                kind: event_kind_name(kind).to_owned(),
                causation_request_id: Some(request_id),
                attempt_id,
                effect_class: effect_class.map(str::to_owned),
                payload,
            },
        )?;
        let (stored_outcome, replayed) = match committed {
            CommandEventOutcome::Recorded { event, outcome } => {
                let event = protocol_event(event)?;
                self.subscriptions.publish(&event);
                (outcome, false)
            }
            CommandEventOutcome::Replay(value) => (value, true),
        };
        Ok((stored_outcome, replayed))
    }
}

fn command_event(
    session_id: Uuid,
    request_id: Uuid,
    occurred_at: OffsetDateTime,
    kind: EventKind,
    payload: Value,
    attempt_id: Option<Uuid>,
    effect_class: Option<&str>,
) -> EventInput {
    EventInput {
        event_id: Uuid::now_v7(),
        session_id,
        occurred_at,
        kind: event_kind_name(kind).to_owned(),
        causation_request_id: Some(request_id),
        attempt_id,
        effect_class: effect_class.map(str::to_owned),
        payload,
    }
}

fn dispatch_events(
    session_id: Uuid,
    request_id: Uuid,
    occurred_at: OffsetDateTime,
    attempt_id: Uuid,
    predecessor: Option<Uuid>,
) -> [EventInput; 2] {
    [
        command_event(
            session_id,
            request_id,
            occurred_at,
            EventKind::DispatchPlanned,
            json!({
                "attempt_id": attempt_id,
                "effect_class": "paid-inference",
                "operation": "provider.prompt",
                "idempotent": false,
                "predecessor_attempt_id": predecessor
            }),
            Some(attempt_id),
            Some("paid-inference"),
        ),
        command_event(
            session_id,
            request_id,
            occurred_at,
            EventKind::DispatchStarted,
            json!({"attempt_id": attempt_id}),
            Some(attempt_id),
            Some("paid-inference"),
        ),
    ]
}

fn latest_started_attempt(history: &[workbench_storage::PersistedEvent]) -> Option<Uuid> {
    let started = history
        .iter()
        .rev()
        .find(|event| event.kind == "dispatch_started")?;
    let attempt_id = started.attempt_id?;
    let finished = history.iter().any(|event| {
        event.sequence > started.sequence
            && event.attempt_id == Some(attempt_id)
            && matches!(
                event.kind.as_str(),
                "session_completed"
                    | "session_failed"
                    | "session_cancelled"
                    | "session_abandoned"
                    | "outcome_unknown"
            )
    });
    (!finished).then_some(attempt_id)
}

fn provider_execution_context(
    history: &[workbench_storage::PersistedEvent],
    preferred_request_id: Uuid,
) -> Result<ProviderExecutionContext, DaemonError> {
    let select = |kind: &str| {
        history
            .iter()
            .rev()
            .find(|event| {
                event.kind == kind && event.causation_request_id == Some(preferred_request_id)
            })
            .or_else(|| history.iter().rev().find(|event| event.kind == kind))
    };
    let routing = select("routing_planned").ok_or(DaemonError::StorageUnavailable)?;
    let input = select("input_recorded").ok_or(DaemonError::StorageUnavailable)?;
    let provider = routing
        .payload
        .get("provider")
        .and_then(Value::as_str)
        .ok_or(DaemonError::StorageUnavailable)?;
    let runtime_model = routing
        .payload
        .get("runtime_model")
        .and_then(Value::as_str)
        .ok_or(DaemonError::StorageUnavailable)?;
    let prompt = input
        .payload
        .get("content")
        .and_then(Value::as_str)
        .ok_or(DaemonError::StorageUnavailable)?;
    Ok(ProviderExecutionContext {
        provider: ProviderId::parse(provider.to_owned())
            .map_err(|_| DaemonError::StorageUnavailable)?,
        runtime_model: runtime_model.to_owned(),
        prompt: prompt.to_owned(),
    })
}

fn fake_provider_capabilities(
    config: &WorkbenchConfiguration,
) -> BTreeMap<String, ConfigProviderCapabilities> {
    config
        .providers
        .iter()
        .filter(|(_, provider)| provider.kind == ProviderType::Fake)
        .map(|(name, _)| {
            (
                name.clone(),
                ConfigProviderCapabilities {
                    adapter_id: name.clone(),
                    adapter_version: DAEMON_VERSION.to_owned(),
                    protocol: "workbench-fake/1".to_owned(),
                    authentication: Authentication::Available,
                    capabilities: vec![
                        Capability::Streaming,
                        Capability::SessionResume,
                        Capability::Cancellation,
                    ],
                    context_window_tokens: None,
                    operations: vec![ProviderOperation {
                        name: "provider.prompt".to_owned(),
                        effect_class: ConfigEffectClass::PaidInference,
                        idempotent: false,
                        material_cost: true,
                        approval: ApprovalMode::Policy,
                    }],
                },
            )
        })
        .collect()
}

fn route_candidate(
    config: &WorkbenchConfiguration,
    resolved: &ResolvedModel,
    permission: DefaultToolMode,
) -> Result<RouteCandidate, DaemonError> {
    let role = config
        .roles
        .get(&resolved.role)
        .ok_or(DaemonError::StorageUnavailable)?;
    RouteCandidate::new(
        "prompt",
        RouteDestination {
            role: RoleId::parse(resolved.role.clone()).map_err(|_| DaemonError::Internal)?,
            model_alias: ModelAlias::parse(resolved.model_alias.clone())
                .map_err(|_| DaemonError::Internal)?,
            provider: ProviderId::parse(resolved.provider.clone())
                .map_err(|_| DaemonError::Internal)?,
            runtime_model: resolved.runtime_model.clone(),
        },
        RouteContext {
            tools: role
                .tools
                .iter()
                .cloned()
                .map(ToolId::parse)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| DaemonError::Internal)?,
            data_sources: role
                .data_sources
                .iter()
                .cloned()
                .map(DataSourceId::parse)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| DaemonError::Internal)?,
            permission: match permission {
                DefaultToolMode::ReadOnly => PermissionScope::ReadOnly,
                DefaultToolMode::ApprovalRequired => PermissionScope::ApprovalRequired,
                DefaultToolMode::Denied => PermissionScope::Denied,
            },
        },
        Risk::Low,
        1.0,
    )
    .map_err(|_| DaemonError::Internal)
}

fn effective_provider_permission(config: &WorkbenchConfiguration) -> DefaultToolMode {
    let provider_prompt =
        ToolId::parse("provider-prompt").expect("static provider prompt identifier is valid");
    let denied_tools = config
        .policies
        .global_deny
        .iter()
        .filter_map(|tool| ToolId::parse(tool.clone()).ok())
        .collect::<BTreeSet<_>>();
    let layer = PolicyLayer {
        source: PolicySource::Session,
        default_mode: match config.policies.default_tool_mode {
            DefaultToolMode::ReadOnly => PermissionMode::ReadOnly,
            DefaultToolMode::ApprovalRequired => PermissionMode::ApprovalRequired,
            DefaultToolMode::Denied => PermissionMode::Denied,
        },
        tool_modes: BTreeMap::new(),
        denied_tools,
    };
    let protected = protect_effect(
        resolve_tool_policy(&provider_prompt, &[layer]),
        EffectClass::PaidInference,
        true,
    );
    match protected.mode {
        PermissionMode::ReadOnly => DefaultToolMode::ReadOnly,
        PermissionMode::ApprovalRequired => DefaultToolMode::ApprovalRequired,
        PermissionMode::Denied => DefaultToolMode::Denied,
    }
}

fn permission_from_scope(scope: PermissionScope) -> DefaultToolMode {
    match scope {
        PermissionScope::ReadOnly => DefaultToolMode::ReadOnly,
        PermissionScope::ApprovalRequired => DefaultToolMode::ApprovalRequired,
        PermissionScope::Denied => DefaultToolMode::Denied,
    }
}

fn routing_payload(plan: &RoutingPlan) -> Value {
    json!({
        "intent": plan.intent,
        "role": plan.destination.role.as_str(),
        "model_alias": plan.destination.model_alias.as_str(),
        "provider": plan.destination.provider.as_str(),
        "runtime_model": plan.destination.runtime_model,
        "context_sources": plan.context.data_sources.iter().map(DataSourceId::as_str).collect::<Vec<_>>(),
        "tools": plan.context.tools.iter().map(ToolId::as_str).collect::<Vec<_>>(),
        "permission_scope": [permission_scope_name(plan.context.permission)],
        "risk": plan.risk,
        "confidence": plan.confidence,
        "selected_by": plan.selected_by
    })
}

const fn permission_scope_name(mode: PermissionScope) -> &'static str {
    match mode {
        PermissionScope::ReadOnly => "read-only",
        PermissionScope::ApprovalRequired => "approval-required",
        PermissionScope::Denied => "denied",
    }
}

const fn selected_rule_name(rule: SelectedRule) -> &'static str {
    match rule {
        SelectedRule::Explicit => "explicit",
        SelectedRule::Workflow => "workflow",
        SelectedRule::Resolver => "resolver",
        SelectedRule::Coordinator => "coordinator",
    }
}

fn durable_failure(error: DaemonError) -> Value {
    json!({ DURABLE_FAILURE_FIELD: error.into_protocol() })
}

fn replay_command_outcome(outcome: Value) -> Result<CommandSuccess, DaemonError> {
    if let Some(failure) = outcome.get(DURABLE_FAILURE_FIELD) {
        let error: ProtocolError =
            serde_json::from_value(failure.clone()).map_err(|_| DaemonError::StorageUnavailable)?;
        Err(DaemonError::Recorded(error))
    } else {
        value_success(outcome)
    }
}

fn success<T: Serialize>(value: T) -> Result<CommandSuccess, DaemonError> {
    value_success(serde_json::to_value(value)?)
}

#[allow(clippy::unnecessary_wraps)]
fn value_success(value: Value) -> Result<CommandSuccess, DaemonError> {
    Ok(CommandSuccess {
        result: value,
        subscription: None,
    })
}

fn session_result(session_id: Uuid, folded: &FoldedSession) -> SessionResult {
    SessionResult {
        session_id,
        state: folded.state,
        last_sequence: folded.last_sequence,
        pending_approval_id: folded.pending_approval_id,
        uncertain_attempt_id: folded.uncertain_attempt_id,
    }
}

fn parse_session_state(state: &str) -> Result<SessionState, DaemonError> {
    match state {
        "ready" => Ok(SessionState::Ready),
        "running" => Ok(SessionState::Running),
        "pausing" => Ok(SessionState::Pausing),
        "paused" => Ok(SessionState::Paused),
        "awaiting_clarification" => Ok(SessionState::AwaitingClarification),
        "awaiting_approval" => Ok(SessionState::AwaitingApproval),
        "cancel_requested" => Ok(SessionState::CancelRequested),
        "outcome_unknown" => Ok(SessionState::OutcomeUnknown),
        "completed" => Ok(SessionState::Completed),
        "failed" => Ok(SessionState::Failed),
        "cancelled" => Ok(SessionState::Cancelled),
        "abandoned" => Ok(SessionState::Abandoned),
        "deleting" => Ok(SessionState::Deleting),
        _ => Err(DaemonError::StorageUnavailable),
    }
}

fn fold_session(
    history: &[workbench_storage::PersistedEvent],
) -> Result<FoldedSession, DaemonError> {
    if history.is_empty() || history[0].kind != "session_created" {
        return Err(DaemonError::SessionNotFound);
    }
    let mut folded = FoldedSession {
        state: SessionState::Ready,
        last_sequence: 0,
        pending_approval_id: None,
        pending_request_id: None,
        uncertain_attempt_id: None,
        approval_decisions: HashMap::new(),
    };
    for event in history {
        if event.sequence != folded.last_sequence + 1 {
            return Err(DaemonError::StorageUnavailable);
        }
        folded.last_sequence = event.sequence;
        match event.kind.as_str() {
            "dispatch_started" | "session_resumed" => folded.state = SessionState::Running,
            "clarification_requested" => folded.state = SessionState::AwaitingClarification,
            "approval_requested" => {
                folded.state = SessionState::AwaitingApproval;
                folded.pending_approval_id = value_uuid(&event.payload, "approval_id");
                folded.pending_request_id = event.causation_request_id;
            }
            "approval_recorded" => {
                let approval_id = value_uuid(&event.payload, "approval_id")
                    .ok_or(DaemonError::StorageUnavailable)?;
                let decision: ApprovalDecision = serde_json::from_value(
                    event
                        .payload
                        .get("decision")
                        .cloned()
                        .ok_or(DaemonError::StorageUnavailable)?,
                )?;
                folded.approval_decisions.insert(approval_id, decision);
                folded.pending_approval_id = None;
                folded.pending_request_id = None;
                folded.state = match decision {
                    ApprovalDecision::Grant => SessionState::Running,
                    ApprovalDecision::Deny => SessionState::Paused,
                };
            }
            "pause_requested" => folded.state = SessionState::Pausing,
            "session_paused" => folded.state = SessionState::Paused,
            "session_redirected" if folded.state == SessionState::AwaitingClarification => {
                folded.state = SessionState::Ready;
            }
            "cancel_requested" => folded.state = SessionState::CancelRequested,
            "outcome_unknown" => {
                folded.state = SessionState::OutcomeUnknown;
                folded.uncertain_attempt_id = event
                    .attempt_id
                    .or_else(|| value_uuid(&event.payload, "attempt_id"));
            }
            "outcome_reconciled" => {
                folded.uncertain_attempt_id = None;
            }
            "session_completed" => folded.state = SessionState::Completed,
            "session_failed" => folded.state = SessionState::Failed,
            "session_cancelled" => folded.state = SessionState::Cancelled,
            "session_abandoned" => folded.state = SessionState::Abandoned,
            "session_deletion_requested" => folded.state = SessionState::Deleting,
            _ => {}
        }
    }
    Ok(folded)
}

fn protocol_event(event: workbench_storage::PersistedEvent) -> Result<SessionEvent, DaemonError> {
    Ok(SessionEvent {
        protocol: workbench_protocol::PROTOCOL_V1.to_owned(),
        event_id: event.event_id,
        session_id: event.session_id,
        sequence: event.sequence,
        causation_request_id: event.causation_request_id,
        kind: parse_event_kind(&event.kind)?,
        occurred_at: format_time(event.occurred_at)?,
        data: event.payload,
    })
}

fn parse_event_kind(kind: &str) -> Result<EventKind, DaemonError> {
    match kind {
        "session_created" => Ok(EventKind::SessionCreated),
        "configuration_resolved" => Ok(EventKind::ConfigurationResolved),
        "input_recorded" => Ok(EventKind::InputRecorded),
        "routing_planned" => Ok(EventKind::RoutingPlanned),
        "clarification_requested" => Ok(EventKind::ClarificationRequested),
        "approval_requested" => Ok(EventKind::ApprovalRequested),
        "approval_recorded" => Ok(EventKind::ApprovalRecorded),
        "dispatch_planned" => Ok(EventKind::DispatchPlanned),
        "dispatch_started" => Ok(EventKind::DispatchStarted),
        "dispatch_acknowledged" => Ok(EventKind::DispatchAcknowledged),
        "provider_event" => Ok(EventKind::ProviderEvent),
        "tool_event" => Ok(EventKind::ToolEvent),
        "pause_requested" => Ok(EventKind::PauseRequested),
        "session_paused" => Ok(EventKind::SessionPaused),
        "session_resumed" => Ok(EventKind::SessionResumed),
        "session_redirected" => Ok(EventKind::SessionRedirected),
        "cancel_requested" => Ok(EventKind::CancelRequested),
        "cancel_confirmed" => Ok(EventKind::CancelConfirmed),
        "outcome_unknown" => Ok(EventKind::OutcomeUnknown),
        "outcome_reconciled" => Ok(EventKind::OutcomeReconciled),
        "session_completed" => Ok(EventKind::SessionCompleted),
        "session_failed" => Ok(EventKind::SessionFailed),
        "session_cancelled" => Ok(EventKind::SessionCancelled),
        "session_abandoned" => Ok(EventKind::SessionAbandoned),
        "session_exported" => Ok(EventKind::SessionExported),
        "session_deletion_requested" => Ok(EventKind::SessionDeletionRequested),
        "session_deleted" => Ok(EventKind::SessionDeleted),
        _ => Err(DaemonError::StorageUnavailable),
    }
}

const fn event_kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::SessionCreated => "session_created",
        EventKind::ConfigurationResolved => "configuration_resolved",
        EventKind::InputRecorded => "input_recorded",
        EventKind::RoutingPlanned => "routing_planned",
        EventKind::ClarificationRequested => "clarification_requested",
        EventKind::ApprovalRequested => "approval_requested",
        EventKind::ApprovalRecorded => "approval_recorded",
        EventKind::DispatchPlanned => "dispatch_planned",
        EventKind::DispatchStarted => "dispatch_started",
        EventKind::DispatchAcknowledged => "dispatch_acknowledged",
        EventKind::ProviderEvent => "provider_event",
        EventKind::ToolEvent => "tool_event",
        EventKind::PauseRequested => "pause_requested",
        EventKind::SessionPaused => "session_paused",
        EventKind::SessionResumed => "session_resumed",
        EventKind::SessionRedirected => "session_redirected",
        EventKind::CancelRequested => "cancel_requested",
        EventKind::CancelConfirmed => "cancel_confirmed",
        EventKind::OutcomeUnknown => "outcome_unknown",
        EventKind::OutcomeReconciled => "outcome_reconciled",
        EventKind::SessionCompleted => "session_completed",
        EventKind::SessionFailed => "session_failed",
        EventKind::SessionCancelled => "session_cancelled",
        EventKind::SessionAbandoned => "session_abandoned",
        EventKind::SessionExported => "session_exported",
        EventKind::SessionDeletionRequested => "session_deletion_requested",
        EventKind::SessionDeleted => "session_deleted",
    }
}

fn terminal_payload(summary: &str) -> Value {
    json!({
        "summary": summary,
        "correlation_id": Uuid::now_v7()
    })
}

fn control_payload(
    control_id: Uuid,
    context: &ClientContext,
    instruction: Option<String>,
) -> Value {
    let mut payload = json!({
        "control_id": control_id,
        "actor": context.actor(),
    });
    if let Some(instruction) = instruction {
        payload["instruction"] = Value::String(instruction);
    }
    payload
}

fn format_time(value: OffsetDateTime) -> Result<String, DaemonError> {
    value.format(&Rfc3339).map_err(|_| DaemonError::Internal)
}

fn value_uuid(value: &Value, field: &str) -> Option<Uuid> {
    value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn uuid_field(value: &Value, field: &'static str) -> Result<Uuid, DaemonError> {
    value_uuid(value, field).ok_or(DaemonError::StorageUnavailable)
}

#[derive(Debug)]
enum DaemonError {
    Recorded(ProtocolError),
    InvalidRequest(&'static str),
    UnsupportedVersion,
    SessionNotFound,
    InvalidTransition,
    CapabilityUnavailable,
    PolicyDenied,
    StorageUnavailable,
    KeyStoreUnavailable,
    ClientLagged,
    Internal,
}

impl DaemonError {
    const fn code_name(&self) -> &'static str {
        match self {
            Self::Recorded(error) => match error.code {
                ErrorCode::InvalidRequest => "invalid_request",
                ErrorCode::UnsupportedVersion => "unsupported_version",
                ErrorCode::FrameTooLarge => "frame_too_large",
                ErrorCode::UnauthorizedPeer => "unauthorized_peer",
                ErrorCode::SessionNotFound => "session_not_found",
                ErrorCode::InvalidTransition => "invalid_transition",
                ErrorCode::CapabilityUnavailable => "capability_unavailable",
                ErrorCode::PolicyDenied => "policy_denied",
                ErrorCode::ApprovalRequired => "approval_required",
                ErrorCode::ProviderUnavailable => "provider_unavailable",
                ErrorCode::ProviderTimeout => "provider_timeout",
                ErrorCode::OutcomeUnknown => "outcome_unknown",
                ErrorCode::ClientLagged => "client_lagged",
                ErrorCode::StorageUnavailable => "storage_unavailable",
                ErrorCode::KeyStoreUnavailable => "key_store_unavailable",
                ErrorCode::Internal => "internal",
            },
            Self::InvalidRequest(_) => "invalid_request",
            Self::UnsupportedVersion => "unsupported_version",
            Self::SessionNotFound => "session_not_found",
            Self::InvalidTransition => "invalid_transition",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::PolicyDenied => "policy_denied",
            Self::StorageUnavailable => "storage_unavailable",
            Self::KeyStoreUnavailable => "key_store_unavailable",
            Self::ClientLagged => "client_lagged",
            Self::Internal => "internal",
        }
    }

    fn into_protocol(self) -> ProtocolError {
        if let Self::Recorded(error) = self {
            return error;
        }
        let (code, message) = match self {
            Self::Recorded(_) => unreachable!("recorded failure returned above"),
            Self::InvalidRequest(message) => (ErrorCode::InvalidRequest, message),
            Self::UnsupportedVersion => (
                ErrorCode::UnsupportedVersion,
                "no compatible protocol version is available",
            ),
            Self::SessionNotFound => (ErrorCode::SessionNotFound, "session was not found"),
            Self::InvalidTransition => (
                ErrorCode::InvalidTransition,
                "session transition is not allowed",
            ),
            Self::CapabilityUnavailable => (
                ErrorCode::CapabilityUnavailable,
                "required provider capability is unavailable",
            ),
            Self::PolicyDenied => (
                ErrorCode::PolicyDenied,
                "effective policy denied the action",
            ),
            Self::StorageUnavailable => (
                ErrorCode::StorageUnavailable,
                "encrypted storage is unavailable",
            ),
            Self::KeyStoreUnavailable => (
                ErrorCode::KeyStoreUnavailable,
                "platform key store is unavailable",
            ),
            Self::ClientLagged => (
                ErrorCode::ClientLagged,
                "client exceeded the bounded event queue",
            ),
            Self::Internal => (ErrorCode::Internal, "internal daemon failure"),
        };
        ProtocolError {
            code,
            message: message.to_owned(),
            retryable: matches!(code, ErrorCode::StorageUnavailable),
            correlation_id: Uuid::now_v7(),
        }
    }
}

impl From<StorageError> for DaemonError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidInput(_) | StorageError::RequestConflict => {
                Self::InvalidRequest("storage rejected the command")
            }
            StorageError::SessionNotFound => Self::SessionNotFound,
            StorageError::KeyStoreUnavailable(_) => Self::KeyStoreUnavailable,
            StorageError::StorageUnavailable(_)
            | StorageError::AuthenticationFailed
            | StorageError::UnsafeExportPath => Self::StorageUnavailable,
        }
    }
}

impl From<workbench_config::ConfigError> for DaemonError {
    fn from(error: workbench_config::ConfigError) -> Self {
        match error {
            workbench_config::ConfigError::CapabilityUnavailable { .. } => {
                Self::CapabilityUnavailable
            }
            workbench_config::ConfigError::Io(_) => Self::StorageUnavailable,
            _ => Self::InvalidRequest("configuration is invalid"),
        }
    }
}

impl From<serde_json::Error> for DaemonError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}

impl From<workbench_protocol::SubscriptionError> for DaemonError {
    fn from(error: workbench_protocol::SubscriptionError) -> Self {
        match error {
            workbench_protocol::SubscriptionError::ClientLagged => Self::ClientLagged,
            _ => Self::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, os::unix::fs::PermissionsExt, pin::Pin, sync::atomic::AtomicUsize};

    use futures_util::stream;
    use tokio::sync::oneshot;
    use workbench_config::{ACP_PROTOCOL, AdapterInput};
    use workbench_core::ports::{AuthenticationStatus, ProviderCapabilities, ProviderStream};
    use workbench_protocol::{
        PROTOCOL_V1,
        command::{CreateSessionParams, EmptyParams, InitializeParams, ListSessionsParams},
    };

    use super::*;
    use crate::telemetry::{RouteRule, TelemetryOutcome};

    type AdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[derive(Clone, Copy)]
    enum AdapterBehavior {
        Complete,
        SetupFailure { definite: bool },
        StreamFailure { definite: bool },
        PendingCancel(CancellationStatus),
        PendingCancelStreamFailure(CancellationStatus),
    }

    struct TestProvider {
        behavior: AdapterBehavior,
        cancel_calls: AtomicUsize,
        pending: Mutex<Option<oneshot::Sender<()>>>,
        seen_prompt: Mutex<Option<ProviderPrompt>>,
    }

    impl TestProvider {
        fn new(behavior: AdapterBehavior) -> Self {
            Self {
                behavior,
                cancel_calls: AtomicUsize::new(0),
                pending: Mutex::new(None),
                seen_prompt: Mutex::new(None),
            }
        }

        fn failure(definite: bool) -> ProviderFailure {
            ProviderFailure {
                category: workbench_core::FailureCategory::ProviderUnavailable,
                user_safe_message: "synthetic provider failure".to_owned(),
                definite,
            }
        }
    }

    impl ProviderAdapter for TestProvider {
        fn capabilities<'life0, 'async_trait>(
            &'life0 self,
        ) -> AdapterFuture<'async_trait, Result<ProviderCapabilities, workbench_core::CoreError>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async {
                Ok(ProviderCapabilities {
                    adapter_id: ProviderId::parse("fake").expect("provider ID"),
                    adapter_version: "1.0.0-test".to_owned(),
                    protocol: ACP_PROTOCOL.to_owned(),
                    authentication: AuthenticationStatus::Available,
                    capabilities: vec![
                        workbench_core::ports::ProviderCapability::Streaming,
                        workbench_core::ports::ProviderCapability::SessionResume,
                        workbench_core::ports::ProviderCapability::Cancellation,
                        workbench_core::ports::ProviderCapability::Acp,
                    ],
                    context_window_tokens: None,
                })
            })
        }

        fn authentication_status<'life0, 'async_trait>(
            &'life0 self,
        ) -> AdapterFuture<'async_trait, Result<AuthenticationStatus, workbench_core::CoreError>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async { Ok(AuthenticationStatus::Available) })
        }

        fn start_session<'life0, 'async_trait>(
            &'life0 self,
        ) -> AdapterFuture<'async_trait, Result<ProviderSessionHandle, ProviderFailure>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                if let AdapterBehavior::SetupFailure { definite } = self.behavior {
                    Err(Self::failure(definite))
                } else {
                    ProviderSessionHandle::new("opaque-test-session")
                        .map_err(|_| Self::failure(true))
                }
            })
        }

        fn resume_session<'life0, 'life1, 'async_trait>(
            &'life0 self,
            opaque_handle: &'life1 str,
        ) -> AdapterFuture<'async_trait, Result<ProviderSessionHandle, ProviderFailure>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                ProviderSessionHandle::new(opaque_handle).map_err(|_| Self::failure(true))
            })
        }

        fn prompt_stream<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _handle: &'life1 ProviderSessionHandle,
            prompt: ProviderPrompt,
        ) -> AdapterFuture<'async_trait, Result<ProviderStream, ProviderFailure>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                *self.seen_prompt.lock().expect("seen prompt") = Some(prompt);
                match self.behavior {
                    AdapterBehavior::Complete => Ok(Box::pin(stream::iter(vec![
                        Ok(ProviderOutput::Acknowledged {
                            provider_request_id: Some("provider-request".to_owned()),
                        }),
                        Ok(ProviderOutput::Content {
                            event_type: "agent_message_chunk".to_owned(),
                            content: NonEmptyText::parse("provider content").expect("content"),
                        }),
                        Ok(ProviderOutput::Tool {
                            event_type: "tool_call".to_owned(),
                            content: NonEmptyText::parse("{\"name\":\"test\"}")
                                .expect("tool content"),
                        }),
                        Ok(ProviderOutput::Completed {
                            summary: "provider completed".to_owned(),
                        }),
                    ])) as ProviderStream),
                    AdapterBehavior::StreamFailure { definite } => Ok(Box::pin(stream::iter(vec![
                        Ok(ProviderOutput::Acknowledged {
                            provider_request_id: None,
                        }),
                        Err(Self::failure(definite)),
                    ]))
                        as ProviderStream),
                    AdapterBehavior::PendingCancel(_) => {
                        let (sender, receiver) = oneshot::channel();
                        *self.pending.lock().expect("pending prompt") = Some(sender);
                        Ok(Box::pin(stream::unfold(Some(receiver), |receiver| async {
                            if let Some(receiver) = receiver {
                                let _ignored = receiver.await;
                            }
                            None::<(
                                Result<ProviderOutput, ProviderFailure>,
                                Option<oneshot::Receiver<()>>,
                            )>
                        })) as ProviderStream)
                    }
                    AdapterBehavior::PendingCancelStreamFailure(_) => {
                        let (sender, receiver) = oneshot::channel();
                        *self.pending.lock().expect("pending prompt") = Some(sender);
                        Ok(Box::pin(stream::once(async move {
                            let _ignored = receiver.await;
                            Err(Self::failure(false))
                        })) as ProviderStream)
                    }
                    AdapterBehavior::SetupFailure { .. } => {
                        unreachable!("setup failure returns before prompt")
                    }
                }
            })
        }

        fn cancel<'life0, 'life1, 'async_trait>(
            &'life0 self,
            _handle: &'life1 ProviderSessionHandle,
            _attempt_id: AttemptId,
        ) -> AdapterFuture<'async_trait, Result<CancellationStatus, workbench_core::CoreError>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                self.cancel_calls.fetch_add(1, Ordering::Relaxed);
                if let Some(sender) = self.pending.lock().expect("pending prompt").take() {
                    let _ignored = sender.send(());
                }
                match self.behavior {
                    AdapterBehavior::PendingCancel(status) => Ok(status),
                    AdapterBehavior::PendingCancelStreamFailure(status) => {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        Ok(status)
                    }
                    _ => Ok(CancellationStatus::Unconfirmed),
                }
            })
        }
    }

    struct TestProviderRegistry {
        adapter: Arc<TestProvider>,
    }

    impl ProviderRegistry for TestProviderRegistry {
        fn adapter(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
            (provider.as_str() == "fake")
                .then(|| Arc::clone(&self.adapter) as Arc<dyn ProviderAdapter>)
        }
    }

    fn client() -> ClientContext {
        ClientContext {
            uid: 1_000,
            client_name: "test-client".to_owned(),
        }
    }

    fn command(command: Command, session_id: Option<Uuid>) -> ClientCommand {
        command_with_request_id(command, session_id, Uuid::now_v7())
    }

    fn command_with_request_id(
        command: Command,
        session_id: Option<Uuid>,
        request_id: Uuid,
    ) -> ClientCommand {
        ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id,
            session_id,
            command,
        }
    }

    async fn create(application: &Arc<Application>) -> Uuid {
        let result = application
            .dispatch(
                command(
                    Command::SessionCreate(CreateSessionParams {
                        persistent: true,
                        configuration_overrides: None,
                    }),
                    None,
                ),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = result.reply else {
            panic!("create failed");
        };
        uuid_field(&result, "session_id").expect("session ID")
    }

    async fn grant_pending(application: &Arc<Application>, session_id: Uuid) {
        let folded = fold_session(&application.history(session_id).expect("approval history"))
            .expect("fold approval");
        let approval_id = folded.pending_approval_id.expect("pending approval");
        let result = application
            .dispatch(
                command(
                    Command::SessionApprovalResolve(ApprovalParams {
                        approval_id,
                        decision: ApprovalDecision::Grant,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        assert!(matches!(result.reply, ServerReply::Success { .. }));
    }

    async fn start_running(application: &Arc<Application>, session_id: Uuid) {
        let prompted = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "start a durable provider attempt".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        assert!(matches!(prompted.reply, ServerReply::Success { .. }));
        grant_pending(application, session_id).await;
        assert_eq!(
            application
                .current_session_state(session_id)
                .expect("materialized running state"),
            SessionState::Running
        );
    }

    fn make_fake_provider_unavailable(application: &Application, session_id: Uuid) {
        let mut configuration = application
            .session_configuration(session_id)
            .expect("configuration");
        configuration
            .providers
            .get_mut("fake")
            .expect("fake provider")
            .kind = ProviderType::Api;
        application
            .session_configs
            .lock()
            .expect("session configurations")
            .insert(session_id, configuration);
    }

    fn startup_from_configuration(configuration: WorkbenchConfiguration) -> StartupConfiguration {
        let sources = vec!["test".to_owned()];
        let snapshot =
            ConfigurationSnapshot::create(&configuration, sources.clone()).expect("snapshot");
        let base_lock = WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new())
            .expect("repository lock");
        StartupConfiguration {
            resolved: configuration,
            snapshot,
            base_lock,
            sources,
            lock_verified: false,
        }
    }

    fn external_application(
        behavior: AdapterBehavior,
    ) -> (Arc<Application>, Arc<TestProvider>, tempfile::TempDir) {
        external_application_with_authentication(behavior, Authentication::Available)
    }

    fn external_application_with_authentication(
        behavior: AdapterBehavior,
        authentication: Authentication,
    ) -> (Arc<Application>, Arc<TestProvider>, tempfile::TempDir) {
        let test_directory = std::env::current_dir().expect("test working directory");
        let directory = tempfile::Builder::new()
            .prefix("workbench-daemon-provider-")
            .tempdir_in(test_directory)
            .expect("adapter directory");
        let executable = directory.path().join("fake-acp");
        std::fs::write(&executable, b"offline fake adapter").expect("adapter executable");
        let mut permissions = std::fs::metadata(&executable)
            .expect("adapter metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).expect("adapter permissions");

        let mut configuration = WorkbenchConfiguration::safe_builtins();
        let provider = configuration
            .providers
            .get_mut("fake")
            .expect("built-in provider");
        provider.kind = ProviderType::Acp;
        provider.executable = Some(executable.to_string_lossy().into_owned());
        let sources = vec!["test".to_owned()];
        let snapshot =
            ConfigurationSnapshot::create(&configuration, sources.clone()).expect("snapshot");
        let adapter_inputs = BTreeMap::from([(
            "fake".to_owned(),
            AdapterInput::acp(&executable, "1.0.0-test").expect("adapter input"),
        )]);
        let base_lock = WorkbenchLock::repository(&configuration, &snapshot, &adapter_inputs)
            .expect("repository lock");
        let startup = StartupConfiguration {
            resolved: configuration,
            snapshot,
            base_lock,
            sources,
            lock_verified: false,
        };
        let adapter = Arc::new(TestProvider::new(behavior));
        let registry: Arc<dyn ProviderRegistry> = Arc::new(TestProviderRegistry {
            adapter: Arc::clone(&adapter),
        });
        let catalog = BTreeMap::from([(
            "fake".to_owned(),
            ConfigProviderCapabilities {
                adapter_id: "fake".to_owned(),
                adapter_version: "1.0.0-test".to_owned(),
                protocol: ACP_PROTOCOL.to_owned(),
                authentication,
                capabilities: vec![
                    Capability::Streaming,
                    Capability::SessionResume,
                    Capability::Cancellation,
                    Capability::Acp,
                ],
                context_window_tokens: None,
                operations: vec![ProviderOperation {
                    name: "provider.prompt".to_owned(),
                    effect_class: ConfigEffectClass::PaidInference,
                    idempotent: false,
                    material_cost: true,
                    approval: ApprovalMode::Policy,
                }],
            },
        )]);
        let storage =
            SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("in-memory storage");
        (
            Application::new_with_providers(
                storage,
                startup,
                FakeBehavior::default(),
                registry,
                catalog,
            ),
            adapter,
            directory,
        )
    }

    async fn wait_for_state(application: &Application, session_id: Uuid, expected: SessionState) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if application
                    .current_session_state(session_id)
                    .expect("session state")
                    == expected
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("terminal state before deadline");
    }

    fn persist_session<K: KeyStore>(
        storage: &mut SqliteStorage<K>,
        startup: &StartupConfiguration,
        session_id: Uuid,
        request_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Value {
        let (_, snapshot, session_lock) = startup.resolve_session(None).expect("session config");
        let configuration_hash = snapshot.content_hash.clone();
        let lock_hash = session_lock.hash().expect("lock hash");
        let outcome = serde_json::to_value(CreateSessionResult {
            session_id,
            configuration_hash: configuration_hash.clone(),
            lock_hash: lock_hash.clone(),
            state: ReadyState::Ready,
        })
        .expect("creation outcome");
        storage
            .create_session(&CreateSession {
                session_id,
                request_id,
                occurred_at,
                request_parameters: json!({"persistent": true}),
                command_outcome: outcome.clone(),
                configuration_snapshot: serde_json::to_value(snapshot).expect("snapshot JSON"),
                lock_snapshot: serde_json::to_value(session_lock).expect("lock JSON"),
                initial_event_payload: json!({
                    "configuration_hash": configuration_hash,
                    "lock_hash": lock_hash
                }),
            })
            .expect("persist session");
        outcome
    }

    fn due_retention_application(now: OffsetDateTime) -> (Arc<Application>, Uuid) {
        let mut configuration = WorkbenchConfiguration::safe_builtins();
        configuration.storage.retention_days = Some(1);
        let startup = startup_from_configuration(configuration);
        let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let session_id = Uuid::now_v7();
        let terminal_at = now - time::Duration::days(2);
        persist_session(
            &mut storage,
            &startup,
            session_id,
            Uuid::now_v7(),
            terminal_at,
        );
        storage
            .append_event(&EventInput {
                event_id: Uuid::now_v7(),
                session_id,
                occurred_at: terminal_at,
                kind: "session_completed".to_owned(),
                causation_request_id: None,
                attempt_id: None,
                effect_class: None,
                payload: terminal_payload("completed"),
            })
            .expect("terminal event");
        let application = Application::new(storage, startup, FakeBehavior::default());
        application.recover().expect("recovery");
        (application, session_id)
    }

    async fn wait_for_maintenance_discovery(application: &Application, session_id: Uuid) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let discovered = application
                    .session_configs
                    .lock()
                    .expect("session configurations")
                    .contains_key(&session_id);
                if discovered {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("maintenance candidate discovery");
    }

    #[test]
    fn audit_actor_uses_only_the_verified_peer_uid() {
        assert_eq!(client().actor(), "local-user:1000");
        let spoofed = ClientContext {
            uid: 1_000,
            client_name: "local-user:0".to_owned(),
        };
        assert_eq!(spoofed.actor(), "local-user:1000");
    }

    #[tokio::test]
    async fn status_marks_non_available_catalog_authentication_unavailable() {
        let (application, _adapter, _directory) = external_application_with_authentication(
            AdapterBehavior::Complete,
            Authentication::InteractiveRequired,
        );
        let status = application
            .dispatch(command(Command::StatusGet(EmptyParams {}), None), &client())
            .await;
        let ServerReply::Success { result, .. } = status.reply else {
            panic!("status failed");
        };
        let status: StatusResult = serde_json::from_value(result).expect("status result");
        assert_eq!(
            status.adapters,
            [AdapterHealth {
                id: "fake".to_owned(),
                status: AdapterStatus::Unavailable,
            }]
        );
    }

    #[tokio::test]
    async fn provider_registry_streams_every_normalized_output_into_one_attempt() {
        let (application, adapter, _directory) = external_application(AdapterBehavior::Complete);
        let session_id = create(&application).await;
        let prompted = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "execute through the provider registry".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        assert!(matches!(prompted.reply, ServerReply::Success { .. }));
        grant_pending(&application, session_id).await;
        wait_for_state(&application, session_id, SessionState::Completed).await;

        let history = application.history(session_id).expect("provider history");
        let attempt_ids = history
            .iter()
            .filter(|event| {
                matches!(
                    event.kind.as_str(),
                    "dispatch_started"
                        | "dispatch_acknowledged"
                        | "provider_event"
                        | "tool_event"
                        | "session_completed"
                )
            })
            .map(|event| event.attempt_id.expect("provider attempt ID"))
            .collect::<BTreeSet<_>>();
        assert_eq!(attempt_ids.len(), 1);
        let normalized = history
            .iter()
            .filter(|event| {
                matches!(
                    event.kind.as_str(),
                    "dispatch_acknowledged" | "provider_event" | "tool_event" | "session_completed"
                )
            })
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            normalized,
            [
                "dispatch_acknowledged",
                "provider_event",
                "tool_event",
                "session_completed"
            ]
        );
        let prompt = adapter
            .seen_prompt
            .lock()
            .expect("seen prompt")
            .clone()
            .expect("provider prompt");
        assert_eq!(prompt.session_id.as_uuid(), session_id);
        assert_eq!(prompt.runtime_model, "deterministic-v1");
        assert_eq!(
            prompt.content.as_str(),
            "execute through the provider registry"
        );
        assert_eq!(
            prompt.attempt_id.as_uuid(),
            *attempt_ids.iter().next().expect("attempt")
        );
    }

    #[tokio::test]
    async fn provider_failures_preserve_definite_and_uncertain_attempt_semantics() {
        for (behavior, expected_state, expected_terminal) in [
            (
                AdapterBehavior::SetupFailure { definite: true },
                SessionState::Failed,
                "session_failed",
            ),
            (
                AdapterBehavior::StreamFailure { definite: false },
                SessionState::OutcomeUnknown,
                "outcome_unknown",
            ),
        ] {
            let (application, _adapter, _directory) = external_application(behavior);
            let session_id = create(&application).await;
            let prompted = application
                .dispatch(
                    command(
                        Command::SessionPrompt(PromptParams {
                            text: "exercise provider failure semantics".to_owned(),
                            explicit_target: None,
                        }),
                        Some(session_id),
                    ),
                    &client(),
                )
                .await;
            assert!(matches!(prompted.reply, ServerReply::Success { .. }));
            grant_pending(&application, session_id).await;
            wait_for_state(&application, session_id, expected_state).await;
            let history = application.history(session_id).expect("failure history");
            assert_eq!(
                history.last().map(|event| event.kind.as_str()),
                Some(expected_terminal)
            );
            assert_eq!(
                history
                    .iter()
                    .filter(|event| event.kind == "dispatch_started")
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn external_pause_is_unavailable_and_cancel_is_sent_once_per_attempt() {
        for (status, expected_state) in [
            (CancellationStatus::Confirmed, SessionState::Cancelled),
            (
                CancellationStatus::Unconfirmed,
                SessionState::OutcomeUnknown,
            ),
        ] {
            let (application, adapter, _directory) =
                external_application(AdapterBehavior::PendingCancel(status));
            let session_id = create(&application).await;
            let prompted = application
                .dispatch(
                    command(
                        Command::SessionPrompt(PromptParams {
                            text: "keep the provider prompt active".to_owned(),
                            explicit_target: None,
                        }),
                        Some(session_id),
                    ),
                    &client(),
                )
                .await;
            assert!(matches!(prompted.reply, ServerReply::Success { .. }));
            grant_pending(&application, session_id).await;
            wait_for_state(&application, session_id, SessionState::Running).await;

            let paused = application
                .dispatch(
                    command(Command::SessionPause(EmptyParams {}), Some(session_id)),
                    &client(),
                )
                .await;
            assert!(matches!(
                paused.reply,
                ServerReply::Failure {
                    error: ProtocolError {
                        code: ErrorCode::CapabilityUnavailable,
                        ..
                    },
                    ..
                }
            ));

            let cancellation_request = Uuid::now_v7();
            let cancel = command_with_request_id(
                Command::SessionCancel(EmptyParams {}),
                Some(session_id),
                cancellation_request,
            );
            let cancelled = application.dispatch(cancel.clone(), &client()).await;
            assert!(matches!(cancelled.reply, ServerReply::Success { .. }));
            wait_for_state(&application, session_id, expected_state).await;
            let replayed = application.dispatch(cancel, &client()).await;
            assert!(matches!(replayed.reply, ServerReply::Success { .. }));
            assert_eq!(adapter.cancel_calls.load(Ordering::Relaxed), 1);

            let history = application.history(session_id).expect("cancel history");
            if status == CancellationStatus::Confirmed {
                let terminal = history
                    .iter()
                    .rev()
                    .take(2)
                    .map(|event| event.kind.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(terminal, ["session_cancelled", "cancel_confirmed"]);
            } else {
                assert_eq!(
                    history.last().map(|event| event.kind.as_str()),
                    Some("outcome_unknown")
                );
                assert!(!history.iter().any(|event| event.kind == "cancel_confirmed"));
            }
        }
    }

    #[tokio::test]
    async fn provider_stream_failure_during_cancel_cannot_override_confirmed_cancellation() {
        let (application, adapter, _directory) = external_application(
            AdapterBehavior::PendingCancelStreamFailure(CancellationStatus::Confirmed),
        );
        let session_id = create(&application).await;
        let prompted = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "cancel while the provider stream fails".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        assert!(matches!(prompted.reply, ServerReply::Success { .. }));
        grant_pending(&application, session_id).await;
        wait_for_state(&application, session_id, SessionState::Running).await;

        let cancelled = application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(cancelled.reply, ServerReply::Success { .. }));
        wait_for_state(&application, session_id, SessionState::Cancelled).await;

        let history = application.history(session_id).expect("cancel history");
        assert!(!history.iter().any(|event| event.kind == "outcome_unknown"));
        assert!(history.windows(2).any(|events| {
            events[0].kind == "cancel_confirmed" && events[1].kind == "session_cancelled"
        }));
        assert_eq!(adapter.cancel_calls.load(Ordering::Relaxed), 1);
        assert!(!application.active.lock().await.contains_key(&session_id));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn session_list_returns_metadata_only_in_descending_cursor_pages() {
        let application = Application::in_memory(
            startup_from_configuration(WorkbenchConfiguration::safe_builtins()),
            FakeBehavior::default(),
        )
        .expect("application");
        let session_ids = [
            create(&application).await,
            create(&application).await,
            create(&application).await,
        ];
        let mut expected = session_ids;
        expected.sort_by_key(|session_id| std::cmp::Reverse(session_id.to_string()));

        let first = application
            .dispatch(
                command(
                    Command::SessionList(ListSessionsParams {
                        limit: 2,
                        before_session_id: None,
                    }),
                    None,
                ),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = first.reply else {
            panic!("first session list failed");
        };
        let first: ListSessionsResult =
            serde_json::from_value(result.clone()).expect("first session list result");
        assert_eq!(
            first
                .sessions
                .iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>(),
            expected[..2]
        );
        assert_eq!(first.next_before_session_id, Some(expected[1]));
        assert!(
            result["sessions"]
                .as_array()
                .expect("session summaries")
                .iter()
                .all(|session| {
                    session.as_object().is_some_and(|session| {
                        (3..=4).contains(&session.len())
                            && session.contains_key("session_id")
                            && session.contains_key("state")
                            && session.contains_key("created_at")
                            && session.keys().all(|field| {
                                matches!(
                                    field.as_str(),
                                    "session_id" | "state" | "created_at" | "terminal_at"
                                )
                            })
                    })
                }),
            "session list must expose metadata fields only"
        );
        for session in &first.sessions {
            assert_eq!(session.state, SessionState::Ready);
            OffsetDateTime::parse(&session.created_at, &Rfc3339).expect("RFC 3339 creation time");
            assert_eq!(session.terminal_at, None);
        }

        let second = application
            .dispatch(
                command(
                    Command::SessionList(ListSessionsParams {
                        limit: 2,
                        before_session_id: first.next_before_session_id,
                    }),
                    None,
                ),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = second.reply else {
            panic!("second session list failed");
        };
        let second: ListSessionsResult =
            serde_json::from_value(result).expect("second session list result");
        assert_eq!(
            second
                .sessions
                .iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>(),
            expected[2..]
        );
        assert_eq!(second.next_before_session_id, None);
        assert_eq!(
            application.history_replays.load(Ordering::Relaxed),
            0,
            "session list must not replay or fold encrypted event histories"
        );
        for session_id in session_ids {
            assert_eq!(
                application
                    .storage
                    .replay(session_id, 0)
                    .expect("session history after listing")
                    .len(),
                1,
                "session list must not append events"
            );
        }
    }

    #[tokio::test]
    async fn restart_loads_pinned_configuration_and_replays_exact_create_result() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("private temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private temporary permissions");
        let database = directory.path().join("workbench.sqlite");
        let keys = MemoryKeyStore::new();
        let mut original_configuration = WorkbenchConfiguration::safe_builtins();
        original_configuration
            .models
            .get_mut("fake-default")
            .expect("default model")
            .runtime_model = "pinned-v1".to_owned();
        let original_startup = startup_from_configuration(original_configuration);
        let request_id = Uuid::now_v7();
        let create_command = ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id,
            session_id: None,
            command: Command::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
            }),
        };
        let first_application = Application::new(
            SqliteStorage::open(&database, keys.clone()).expect("first storage"),
            original_startup,
            FakeBehavior::default(),
        );
        let first = first_application
            .dispatch(create_command.clone(), &client())
            .await;
        let ServerReply::Success {
            result: first_result,
            ..
        } = first.reply
        else {
            panic!("first create failed");
        };
        let session_id = uuid_field(&first_result, "session_id").expect("session");
        drop(first_application);

        let mut changed_configuration = WorkbenchConfiguration::safe_builtins();
        changed_configuration
            .models
            .get_mut("fake-default")
            .expect("default model")
            .runtime_model = "startup-v2".to_owned();
        let restarted = Application::new(
            SqliteStorage::open(&database, keys).expect("reopened storage"),
            startup_from_configuration(changed_configuration),
            FakeBehavior::default(),
        );
        restarted.recover().expect("restart recovery");

        assert_eq!(
            restarted
                .session_configuration(session_id)
                .expect("pinned configuration")
                .models["fake-default"]
                .runtime_model,
            "pinned-v1"
        );
        assert_eq!(
            restarted.pinned_locks.lock().expect("pinned locks")[&session_id]
                .hash()
                .expect("pinned lock hash"),
            first_result["lock_hash"]
                .as_str()
                .expect("result lock hash")
        );
        let replay = restarted.dispatch(create_command, &client()).await;
        let ServerReply::Success {
            result: replay_result,
            ..
        } = replay.reply
        else {
            panic!("create replay failed");
        };
        assert_eq!(replay_result, first_result);
    }

    #[tokio::test]
    async fn policy_failure_is_durable_and_replays_the_same_protocol_error() {
        let mut configuration = WorkbenchConfiguration::safe_builtins();
        configuration.policies.default_tool_mode = DefaultToolMode::Denied;
        let application = Application::in_memory(
            startup_from_configuration(configuration),
            FakeBehavior::default(),
        )
        .expect("application");
        let session_id = create(&application).await;
        let prompt = ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id: Uuid::now_v7(),
            session_id: Some(session_id),
            command: Command::SessionPrompt(PromptParams {
                text: "must remain denied".to_owned(),
                explicit_target: None,
            }),
        };

        let first = application.dispatch(prompt.clone(), &client()).await;
        let ServerReply::Failure {
            error: first_error, ..
        } = first.reply
        else {
            panic!("denied prompt succeeded");
        };
        let history = application.history(session_id).expect("denied history");
        let replay = application.dispatch(prompt, &client()).await;
        let ServerReply::Failure {
            error: replay_error,
            ..
        } = replay.reply
        else {
            panic!("denied replay succeeded");
        };

        assert_eq!(first_error.code, ErrorCode::PolicyDenied);
        assert_eq!(replay_error, first_error);
        assert_eq!(
            application.history(session_id).expect("replayed history"),
            history
        );
        assert!(!history.iter().any(|event| event.kind == "dispatch_started"));
    }

    #[tokio::test]
    async fn preflight_selects_a_compatible_fake_fallback() {
        let mut configuration = WorkbenchConfiguration::safe_builtins();
        let primary = configuration
            .providers
            .get_mut("fake")
            .expect("primary provider");
        primary.kind = ProviderType::Api;
        primary.credential_ref = Some("keychain:test".to_owned());
        primary.privacy = Some(workbench_config::model::Privacy {
            zero_data_retention: true,
            data_collection: workbench_config::model::DataCollection::Deny,
        });
        configuration.providers.insert(
            "fallback".to_owned(),
            workbench_config::model::Provider {
                kind: ProviderType::Fake,
                driver: None,
                executable: None,
                credential_ref: None,
                privacy: None,
            },
        );
        configuration.models.insert(
            "fallback-model".to_owned(),
            workbench_config::model::Model {
                provider: "fallback".to_owned(),
                runtime_model: "fallback-v1".to_owned(),
            },
        );
        configuration
            .roles
            .get_mut("workspace-coordinator")
            .expect("coordinator role")
            .fallback_models
            .push("fallback-model".to_owned());
        configuration
            .roles
            .get_mut("workspace-coordinator")
            .expect("coordinator role")
            .required_capabilities
            .push(Capability::Streaming);
        let application = Application::in_memory(
            startup_from_configuration(configuration),
            FakeBehavior::default(),
        )
        .expect("application");
        let session_id = create(&application).await;

        let result = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "use fallback".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;

        assert!(matches!(result.reply, ServerReply::Success { .. }));
        let routing = application
            .history(session_id)
            .expect("fallback history")
            .into_iter()
            .find(|event| event.kind == "routing_planned")
            .expect("routing event");
        assert_eq!(routing.payload["provider"], "fallback");
        assert_eq!(routing.payload["model_alias"], "fallback-model");
    }

    #[tokio::test]
    async fn fake_adapter_does_not_claim_unimplemented_capabilities() {
        for required in [Capability::Vision, Capability::Mcp, Capability::Acp] {
            let mut configuration = WorkbenchConfiguration::safe_builtins();
            configuration
                .roles
                .get_mut("workspace-coordinator")
                .expect("coordinator role")
                .required_capabilities = vec![required];
            let application = Application::in_memory(
                startup_from_configuration(configuration),
                FakeBehavior::default(),
            )
            .expect("application");
            let session_id = create(&application).await;
            let prompt = command(
                Command::SessionPrompt(PromptParams {
                    text: "requires an unavailable capability".to_owned(),
                    explicit_target: None,
                }),
                Some(session_id),
            );
            let result = application.dispatch(prompt.clone(), &client()).await;
            let ServerReply::Failure {
                error: first_error, ..
            } = result.reply
            else {
                panic!("unavailable capability was accepted");
            };
            assert_eq!(first_error.code, ErrorCode::CapabilityUnavailable);
            let history = application.history(session_id).expect("history");
            assert_eq!(
                fold_session(&history).expect("fold").state,
                SessionState::Ready
            );
            let replay = application.dispatch(prompt, &client()).await;
            let ServerReply::Failure {
                error: replay_error,
                ..
            } = replay.reply
            else {
                panic!("unavailable capability replay was accepted");
            };
            assert_eq!(replay_error, first_error);
            assert_eq!(application.history(session_id).expect("replay"), history);
            assert!(
                !history
                    .iter()
                    .any(|event| event.kind == "clarification_requested")
            );
            assert!(!history.iter().any(|event| event.kind == "routing_planned"));
            assert!(!history.iter().any(|event| event.kind == "dispatch_started"));
        }
    }

    #[tokio::test]
    async fn maintenance_applies_each_sessions_pinned_retention() {
        let keys = MemoryKeyStore::new();
        let mut storage = SqliteStorage::open_in_memory(keys).expect("storage");
        let now = OffsetDateTime::now_utc();
        let old = now - time::Duration::days(3);
        let mut retained_configuration = WorkbenchConfiguration::safe_builtins();
        retained_configuration.storage.retention_days = Some(1);
        let retained_startup = startup_from_configuration(retained_configuration);
        let default_startup = startup_from_configuration(WorkbenchConfiguration::safe_builtins());
        let retained = Uuid::now_v7();
        let default = Uuid::now_v7();
        persist_session(
            &mut storage,
            &retained_startup,
            retained,
            Uuid::now_v7(),
            old,
        );
        persist_session(&mut storage, &default_startup, default, Uuid::now_v7(), old);
        for session_id in [retained, default] {
            storage
                .append_event(&EventInput {
                    event_id: Uuid::now_v7(),
                    session_id,
                    occurred_at: old,
                    kind: "session_completed".to_owned(),
                    causation_request_id: None,
                    attempt_id: None,
                    effect_class: None,
                    payload: terminal_payload("completed"),
                })
                .expect("terminal event");
        }
        let application = Application::new(storage, default_startup, FakeBehavior::default());

        application.recover().expect("recovery");
        assert!(!application.storage.is_deleted(retained).expect("retained"));

        assert_eq!(
            application
                .run_maintenance(now)
                .await
                .expect("retention maintenance"),
            1
        );

        assert!(application.storage.is_deleted(retained).expect("retained"));
        assert!(!application.storage.is_deleted(default).expect("default"));
        assert!(application.session_configuration(default).is_ok());
    }

    #[tokio::test]
    async fn maintenance_skips_a_pending_export_before_evicting_runtime_state() {
        let now = OffsetDateTime::now_utc();
        let (application, session_id) = due_retention_application(now);
        let directory = tempfile::tempdir().expect("export directory");
        let output = directory.path().join("session.age");
        let export_id = Uuid::now_v7();
        std::fs::create_dir(
            directory
                .path()
                .join(format!(".workbench-export-{export_id}.age.part")),
        )
        .expect("unsafe staging collision");
        let recipient = age::x25519::Identity::generate().to_public().to_string();
        let recipients = vec![recipient];
        let parameters = json!({
            "age_recipients": recipients,
            "output_path": output.to_str().expect("UTF-8 output")
        });
        let export = application.storage.execute_export(&ExportCommand {
            session_id,
            request_id: Uuid::now_v7(),
            export_id,
            occurred_at: now,
            parameters,
            output_path: output,
            age_recipients: recipients,
            outcome: json!({"export_id": export_id}),
            event_payload: json!({"export_id": export_id}),
        });
        assert!(matches!(export, Err(StorageError::UnsafeExportPath)));
        let mut subscription = application
            .dispatch(
                command(
                    Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
                    Some(session_id),
                ),
                &client(),
            )
            .await
            .subscription
            .expect("subscription");

        assert_eq!(
            application
                .run_maintenance(now)
                .await
                .expect("pending export is a skipped candidate"),
            0
        );
        assert!(
            !application
                .storage
                .is_deleted(session_id)
                .expect("deletion state")
        );
        assert!(application.session_configuration(session_id).is_ok());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), subscription.next())
                .await
                .expect("queued replay remains available")
                .is_some(),
            "retention must not purge subscribers before deletion preflight"
        );
    }

    #[tokio::test]
    async fn failed_retention_deletion_keeps_runtime_state_until_a_durable_retry() {
        let now = OffsetDateTime::now_utc();
        let (application, session_id) = due_retention_application(now);
        let mut subscription = application
            .dispatch(
                command(
                    Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
                    Some(session_id),
                ),
                &client(),
            )
            .await
            .subscription
            .expect("subscription");
        application
            .fail_next_deletion_request
            .store(true, Ordering::Release);

        let failure = application
            .run_maintenance(now)
            .await
            .expect_err("injected deletion failure");
        assert!(matches!(failure, StorageError::StorageUnavailable(_)));
        assert!(!application.storage.is_deleted(session_id).expect("session"));
        assert!(application.session_configuration(session_id).is_ok());
        assert!(
            subscription.next().await.is_some(),
            "failed retention must not purge existing subscribers"
        );

        assert_eq!(
            application
                .run_maintenance(now)
                .await
                .expect("durable retry"),
            1
        );
        assert!(application.storage.is_deleted(session_id).expect("session"));
        assert!(subscription.next().await.is_none());
    }

    #[tokio::test]
    async fn maintenance_waits_for_accepted_get_export_or_delete_under_the_session_lock() {
        let now = OffsetDateTime::now_utc();
        let (application, session_id) = due_retention_application(now);
        let session_lock = application.session_lock(session_id).await;
        let guard = session_lock.lock().await;
        application
            .evict_session(session_id)
            .expect("clear discovery marker");
        let maintenance_application = Arc::clone(&application);
        let mut maintenance =
            tokio::spawn(async move { maintenance_application.run_maintenance(now).await });

        wait_for_maintenance_discovery(&application, session_id).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut maintenance)
                .await
                .is_err(),
            "maintenance must wait while an accepted session command owns the lock"
        );
        assert!(
            !application
                .storage
                .is_deleted(session_id)
                .expect("deletion state"),
            "maintenance must not interrupt the accepted command"
        );

        drop(guard);
        assert_eq!(
            maintenance
                .await
                .expect("maintenance task")
                .expect("maintenance pass"),
            1
        );
        assert!(
            application
                .storage
                .is_deleted(session_id)
                .expect("deletion state")
        );
    }

    #[tokio::test]
    async fn maintenance_revalidates_a_stale_candidate_after_acquiring_the_session_lock() {
        let now = OffsetDateTime::now_utc();
        let (application, session_id) = due_retention_application(now);
        let session_lock = application.session_lock(session_id).await;
        let guard = session_lock.lock().await;
        application
            .evict_session(session_id)
            .expect("clear discovery marker");
        let maintenance_application = Arc::clone(&application);
        let maintenance =
            tokio::spawn(async move { maintenance_application.run_maintenance(now).await });

        wait_for_maintenance_discovery(&application, session_id).await;
        application
            .storage
            .request_deletion(
                session_id,
                Uuid::now_v7(),
                Uuid::now_v7(),
                now,
                "local-user:1000",
            )
            .expect("accepted deletion");
        application
            .evict_session(session_id)
            .expect("evict deleted session");
        application
            .subscriptions
            .purge_session(session_id)
            .expect("purge deleted session");

        drop(guard);
        assert_eq!(
            maintenance
                .await
                .expect("maintenance task")
                .expect("maintenance pass"),
            0,
            "the stale candidate must not be deleted a second time"
        );
    }

    #[tokio::test]
    async fn no_op_controls_record_exact_outcomes_and_reject_cross_method_request_reuse() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        start_running(&application, session_id).await;
        let paused = application
            .dispatch(
                command(Command::SessionPause(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(paused.reply, ServerReply::Success { .. }));

        let pause_request_id = Uuid::now_v7();
        let no_op_pause = command_with_request_id(
            Command::SessionPause(EmptyParams {}),
            Some(session_id),
            pause_request_id,
        );
        let first_pause = application
            .dispatch(no_op_pause.clone(), &client())
            .await
            .reply;
        let replayed_pause = application.dispatch(no_op_pause, &client()).await.reply;
        assert_eq!(replayed_pause, first_pause);
        let conflicting_resume = application
            .dispatch(
                command_with_request_id(
                    Command::SessionResume(EmptyParams {}),
                    Some(session_id),
                    pause_request_id,
                ),
                &client(),
            )
            .await;
        assert!(matches!(
            conflicting_resume.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::InvalidRequest,
                    ..
                },
                ..
            }
        ));

        let resumed = application
            .dispatch(
                command(Command::SessionResume(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(resumed.reply, ServerReply::Success { .. }));
        let resume_request_id = Uuid::now_v7();
        let no_op_resume = command_with_request_id(
            Command::SessionResume(EmptyParams {}),
            Some(session_id),
            resume_request_id,
        );
        let first_resume = application
            .dispatch(no_op_resume.clone(), &client())
            .await
            .reply;
        let replayed_resume = application.dispatch(no_op_resume, &client()).await.reply;
        assert_eq!(replayed_resume, first_resume);
        let conflicting_pause = application
            .dispatch(
                command_with_request_id(
                    Command::SessionPause(EmptyParams {}),
                    Some(session_id),
                    resume_request_id,
                ),
                &client(),
            )
            .await;
        assert!(matches!(
            conflicting_pause.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::InvalidRequest,
                    ..
                },
                ..
            }
        ));
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn repeated_approval_records_and_replays_the_original_outcome() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_millis(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "request approval".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        let approval_id = fold_session(
            &application
                .storage
                .replay(session_id, 0)
                .expect("approval history"),
        )
        .expect("approval fold")
        .pending_approval_id
        .expect("pending approval");
        let first = application
            .dispatch(
                command(
                    Command::SessionApprovalResolve(ApprovalParams {
                        approval_id,
                        decision: ApprovalDecision::Grant,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        let ServerReply::Success {
            result: original_outcome,
            ..
        } = first.reply
        else {
            panic!("approval failed");
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if application
                    .current_session_state(session_id)
                    .expect("materialized state")
                    == SessionState::Completed
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("provider completion");

        let repeated_request_id = Uuid::now_v7();
        let repeated = command_with_request_id(
            Command::SessionApprovalResolve(ApprovalParams {
                approval_id,
                decision: ApprovalDecision::Grant,
            }),
            Some(session_id),
            repeated_request_id,
        );
        let duplicate = application.dispatch(repeated.clone(), &client()).await;
        let ServerReply::Success {
            result: duplicate_outcome,
            ..
        } = duplicate.reply
        else {
            panic!("duplicate approval failed");
        };
        assert_eq!(duplicate_outcome, original_outcome);
        let replay = application.dispatch(repeated, &client()).await;
        assert!(matches!(
            replay.reply,
            ServerReply::Success { result, .. } if result == original_outcome
        ));
        let conflict = application
            .dispatch(
                command_with_request_id(
                    Command::SessionPause(EmptyParams {}),
                    Some(session_id),
                    repeated_request_id,
                ),
                &client(),
            )
            .await;
        assert!(matches!(
            conflict.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::InvalidRequest,
                    ..
                },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn recovered_materialized_state_keeps_control_ack_independent_of_history_length() {
        let startup = StartupConfiguration::safe_builtins().expect("startup");
        let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let session_id = Uuid::now_v7();
        persist_session(
            &mut storage,
            &startup,
            session_id,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
        );
        for _ in 0..667 {
            let request_id = Uuid::now_v7();
            storage
                .append_events(&[
                    command_event(
                        session_id,
                        request_id,
                        OffsetDateTime::now_utc(),
                        EventKind::SessionResumed,
                        json!({}),
                        None,
                        None,
                    ),
                    command_event(
                        session_id,
                        request_id,
                        OffsetDateTime::now_utc(),
                        EventKind::PauseRequested,
                        json!({}),
                        None,
                        None,
                    ),
                    command_event(
                        session_id,
                        request_id,
                        OffsetDateTime::now_utc(),
                        EventKind::SessionPaused,
                        json!({}),
                        None,
                        None,
                    ),
                ])
                .expect("control history");
        }
        storage
            .append_event(&command_event(
                session_id,
                Uuid::now_v7(),
                OffsetDateTime::now_utc(),
                EventKind::SessionResumed,
                json!({}),
                None,
                None,
            ))
            .expect("final running state");
        let application = Application::new(storage, startup, FakeBehavior::default());
        application.recover().expect("recovery");
        let history = application
            .storage
            .replay(session_id, 0)
            .expect("recovered history");
        assert_eq!(
            application
                .current_session_state(session_id)
                .expect("materialized state"),
            fold_session(&history).expect("folded state").state
        );
        application.history_replays.store(0, Ordering::Relaxed);

        for control in [
            Command::SessionPause(EmptyParams {}),
            Command::SessionResume(EmptyParams {}),
        ] {
            let result = application
                .dispatch(command(control, Some(session_id)), &client())
                .await;
            assert!(matches!(result.reply, ServerReply::Success { .. }));
        }
        assert_eq!(
            application.history_replays.load(Ordering::Relaxed),
            0,
            "control acknowledgement must use the O(1) materialized state"
        );
        assert_eq!(
            application
                .current_session_state(session_id)
                .expect("final materialized state"),
            SessionState::Running
        );
    }

    #[tokio::test]
    async fn cancel_without_an_external_attempt_is_terminal_even_when_provider_would_timeout() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                confirms_cancellation: false,
                cancellation_deadline: Duration::from_millis(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "await approval".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;

        let cancelled = application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = cancelled.reply else {
            panic!("cancel failed");
        };

        assert_eq!(result["state"], "cancelled");
        let history = application.history(session_id).expect("cancel history");
        assert_eq!(
            fold_session(&history).expect("cancel fold").state,
            SessionState::Cancelled
        );
        assert!(!history.iter().any(|event| event.kind == "outcome_unknown"));
    }

    #[tokio::test]
    async fn missing_active_execution_never_confirms_an_attempt_cancellation() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                confirms_cancellation: true,
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        start_running(&application, session_id).await;
        application.active.lock().await.remove(&session_id);

        let cancelled = application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(cancelled.reply, ServerReply::Success { .. }));
        wait_for_state(&application, session_id, SessionState::OutcomeUnknown).await;

        let history = application.history(session_id).expect("cancel history");
        assert!(!history.iter().any(|event| event.kind == "cancel_confirmed"));
        assert_eq!(
            history.last().map(|event| event.kind.as_str()),
            Some("outcome_unknown")
        );
    }

    #[tokio::test]
    async fn failed_cancel_commit_preserves_active_execution_until_a_durable_outcome() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        start_running(&application, session_id).await;
        application
            .fail_next_command_commit
            .store(true, Ordering::Release);

        let cancelled = application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(
            cancelled.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::StorageUnavailable,
                    ..
                },
                ..
            }
        ));
        assert!(application.active.lock().await.contains_key(&session_id));
        let history = application
            .storage
            .replay(session_id, 0)
            .expect("history after failed commit");
        assert_eq!(
            fold_session(&history).expect("running fold").state,
            SessionState::Running
        );
        assert!(!history.iter().any(|event| event.kind == "cancel_requested"));

        application
            .finish_fake(session_id)
            .await
            .expect("tracked execution can still finish");
        assert_eq!(
            application
                .current_session_state(session_id)
                .expect("completed state"),
            SessionState::Completed
        );
    }

    #[tokio::test]
    async fn confirmed_cancellation_persists_an_adjacent_terminal_pair() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                confirms_cancellation: true,
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        start_running(&application, session_id).await;
        application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if application
                    .current_session_state(session_id)
                    .expect("materialized state")
                    == SessionState::Cancelled
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancellation resolution");
        let history = application
            .storage
            .replay(session_id, 0)
            .expect("cancelled history");
        assert!(history.windows(2).any(|events| {
            events[0].kind == "cancel_confirmed"
                && events[1].kind == "session_cancelled"
                && events[0].attempt_id == events[1].attempt_id
        }));
        assert!(!application.active.lock().await.contains_key(&session_id));
    }

    #[test]
    fn recovery_completes_a_legacy_confirmed_cancellation_without_uncertainty() {
        let startup = StartupConfiguration::safe_builtins().expect("startup");
        let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let session_id = Uuid::now_v7();
        persist_session(
            &mut storage,
            &startup,
            session_id,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
        );
        let attempt_id = Uuid::now_v7();
        for event in dispatch_events(
            session_id,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
            attempt_id,
            None,
        ) {
            storage.append_event(&event).expect("dispatch fact");
        }
        let cancel_request = Uuid::now_v7();
        storage
            .append_events(&[
                command_event(
                    session_id,
                    cancel_request,
                    OffsetDateTime::now_utc(),
                    EventKind::CancelRequested,
                    json!({"control_id": cancel_request, "actor": "local-user:1000"}),
                    attempt_id.into(),
                    None,
                ),
                command_event(
                    session_id,
                    cancel_request,
                    OffsetDateTime::now_utc(),
                    EventKind::CancelConfirmed,
                    json!({"control_id": cancel_request}),
                    attempt_id.into(),
                    None,
                ),
            ])
            .expect("legacy partial cancellation");
        let application = Application::new(storage, startup, FakeBehavior::default());

        application.recover().expect("recovery");

        let history = application
            .storage
            .replay(session_id, 0)
            .expect("recovered history");
        assert_eq!(
            fold_session(&history).expect("recovered fold").state,
            SessionState::Cancelled
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| event.kind == "session_cancelled")
                .count(),
            1
        );
        assert!(!history.iter().any(|event| event.kind == "outcome_unknown"));
    }

    #[tokio::test]
    async fn shutdown_drains_accepted_work_and_prevents_later_terminal_events() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        start_running(&application, session_id).await;
        let accepted_guard = application.lifecycle_gate.read().await;
        let shutdown_application = Arc::clone(&application);
        let mut shutdown =
            tokio::spawn(async move { shutdown_application.prepare_shutdown().await });

        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
                .await
                .is_err(),
            "shutdown must drain accepted work before recording uncertainty"
        );
        assert!(!application.shutting_down.load(Ordering::Acquire));
        drop(accepted_guard);
        shutdown
            .await
            .expect("shutdown task")
            .expect("durable shutdown");
        let before_completion = application
            .storage
            .replay(session_id, 0)
            .expect("shutdown history");
        assert_eq!(
            fold_session(&before_completion)
                .expect("shutdown fold")
                .state,
            SessionState::OutcomeUnknown
        );

        application
            .finish_fake(session_id)
            .await
            .expect("late completion is suppressed");
        assert_eq!(
            application
                .storage
                .replay(session_id, 0)
                .expect("history after late completion"),
            before_completion
        );
        let rejected = application
            .dispatch(
                command(Command::SessionGet(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        assert!(matches!(
            rejected.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::InvalidRequest,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn restart_never_confirms_an_active_cancellation_from_fake_behavior() {
        let startup = StartupConfiguration::safe_builtins().expect("startup");
        let mut storage = SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("storage");
        let session_id = Uuid::now_v7();
        persist_session(
            &mut storage,
            &startup,
            session_id,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
        );
        let attempt_id = Uuid::now_v7();
        for event in dispatch_events(
            session_id,
            Uuid::now_v7(),
            OffsetDateTime::now_utc(),
            attempt_id,
            None,
        ) {
            storage.append_event(&event).expect("dispatch fact");
        }
        let cancel_request = Uuid::now_v7();
        storage
            .commit_command_event(
                cancel_request,
                "session.cancel",
                &json!({}),
                &serde_json::to_value(ControlResult {
                    control_id: cancel_request,
                    control: Control::Cancel,
                    state: SessionState::CancelRequested,
                })
                .expect("cancel outcome"),
                &command_event(
                    session_id,
                    cancel_request,
                    OffsetDateTime::now_utc(),
                    EventKind::CancelRequested,
                    json!({
                        "control_id": cancel_request,
                        "actor": "local-user:1000"
                    }),
                    None,
                    None,
                ),
            )
            .expect("durable cancel");
        let application = Application::new(storage, startup, FakeBehavior::default());

        application.recover().expect("cancel recovery");

        let history = application.history(session_id).expect("recovered history");
        let folded = fold_session(&history).expect("recovered fold");
        assert_eq!(folded.state, SessionState::OutcomeUnknown);
        assert_eq!(folded.uncertain_attempt_id, Some(attempt_id));
        assert!(!history.iter().any(|event| event.kind == "cancel_confirmed"));
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn delete_purges_runtime_plaintext_and_create_replays_from_tombstone() {
        let keys = MemoryKeyStore::new();
        let application = Application::new(
            SqliteStorage::open_in_memory(keys.clone()).expect("storage"),
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior::default(),
        );
        let create_command = ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id: Uuid::now_v7(),
            session_id: None,
            command: Command::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
            }),
        };
        let created = application
            .dispatch(create_command.clone(), &client())
            .await;
        let ServerReply::Success {
            result: create_result,
            ..
        } = created.reply
        else {
            panic!("create failed");
        };
        let session_id = uuid_field(&create_result, "session_id").expect("session");
        application
            .append(
                session_id,
                None,
                EventKind::SessionCompleted,
                terminal_payload("ready to delete"),
                None,
                None,
            )
            .expect("terminal event");
        let mut subscription = application
            .dispatch(
                command(
                    Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
                    Some(session_id),
                ),
                &client(),
            )
            .await
            .subscription
            .expect("subscription");
        let deletion_lock = application.session_lock(session_id).await;

        let delete_command = command(
            Command::SessionDelete(DeleteParams {
                confirm_session_id: session_id,
            }),
            Some(session_id),
        );
        application
            .fail_next_deletion_request
            .store(true, Ordering::Release);
        let failed = application
            .dispatch(delete_command.clone(), &client())
            .await;
        assert!(matches!(
            failed.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::StorageUnavailable,
                    ..
                },
                ..
            }
        ));
        assert!(!application.storage.is_deleted(session_id).expect("session"));
        assert!(application.session_configuration(session_id).is_ok());
        assert!(
            subscription.next().await.is_some(),
            "failed deletion must not purge existing subscribers"
        );

        let deleted = application.dispatch(delete_command, &client()).await;
        assert!(matches!(deleted.reply, ServerReply::Success { .. }));
        assert!(Arc::ptr_eq(
            &deletion_lock,
            &application.session_lock(session_id).await
        ));
        assert!(subscription.next().await.is_none());
        assert!(
            !application
                .session_configs
                .lock()
                .expect("configurations")
                .contains_key(&session_id)
        );
        assert!(
            !application
                .pinned_locks
                .lock()
                .expect("locks")
                .contains_key(&session_id)
        );
        assert!(
            application
                .storage
                .is_deleted(session_id)
                .expect("tombstone")
        );
        assert!(
            keys.list("workbench/storage/")
                .expect("key envelopes")
                .iter()
                .all(|key_id| !key_id.contains("/session/"))
        );

        let replay = application.dispatch(create_command, &client()).await;
        let ServerReply::Success {
            result: replay_result,
            ..
        } = replay.reply
        else {
            panic!("deleted create replay failed");
        };
        assert_eq!(replay_result, create_result);
        assert!(
            application
                .storage
                .load_sessions()
                .expect("sessions")
                .is_empty()
        );
        assert!(
            keys.list("workbench/storage/")
                .expect("key envelopes")
                .iter()
                .all(|key_id| !key_id.contains("/session/"))
        );
    }

    #[tokio::test]
    async fn executes_fake_prompt_and_replays_events() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_millis(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        let prompt = command(
            Command::SessionPrompt(PromptParams {
                text: "hello".to_owned(),
                explicit_target: None,
            }),
            Some(session_id),
        );
        let result = application.dispatch(prompt, &client()).await;
        assert!(
            matches!(&result.reply, ServerReply::Success { .. }),
            "{:?}",
            result.reply
        );
        grant_pending(&application, session_id).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        let result = application
            .dispatch(
                command(Command::SessionGet(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = result.reply else {
            panic!("get failed");
        };
        assert_eq!(result["state"], "completed");
        assert!(result["last_sequence"].as_u64().expect("sequence") >= 7);
    }

    #[tokio::test]
    async fn records_bounded_default_explicit_and_completed_paths() {
        let telemetry = Arc::new(BoundedTelemetry::default());
        let application = Application::in_memory_with_telemetry(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_millis(1),
                ..FakeBehavior::default()
            },
            telemetry.clone(),
        )
        .expect("application");
        for explicit_target in [None, Some("workspace-coordinator".to_owned())] {
            let session_id = create(&application).await;
            let result = application
                .dispatch(
                    command(
                        Command::SessionPrompt(PromptParams {
                            text: "bounded telemetry".to_owned(),
                            explicit_target,
                        }),
                        Some(session_id),
                    ),
                    &client(),
                )
                .await;
            assert!(matches!(result.reply, ServerReply::Success { .. }));
            grant_pending(&application, session_id).await;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;

        let snapshot = telemetry.snapshot();
        assert_eq!(
            snapshot.route_decisions(RouteRule::Coordinator, TelemetryOutcome::Success),
            1
        );
        assert_eq!(
            snapshot.route_decisions(RouteRule::Explicit, TelemetryOutcome::Success),
            1
        );
        assert_eq!(snapshot.attempts(TelemetryOutcome::Success), 2);
        assert_eq!(snapshot.rejected_records(), 0);
    }

    #[tokio::test]
    async fn records_bounded_preflight_and_clarification_failures() {
        let telemetry = Arc::new(BoundedTelemetry::default());
        let application = Application::in_memory_with_telemetry(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior::default(),
            telemetry.clone(),
        )
        .expect("application");
        let session_id = create(&application).await;
        make_fake_provider_unavailable(&application, session_id);

        let preflight = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "preflight".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        assert!(matches!(
            preflight.reply,
            ServerReply::Failure {
                error: ProtocolError {
                    code: ErrorCode::CapabilityUnavailable,
                    ..
                },
                ..
            }
        ));
        assert_eq!(
            fold_session(&application.history(session_id).expect("preflight history"))
                .expect("preflight fold")
                .state,
            SessionState::Ready
        );
        let explicit_session = create(&application).await;
        make_fake_provider_unavailable(&application, explicit_session);
        let clarification = application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "clarification".to_owned(),
                        explicit_target: Some("missing-role".to_owned()),
                    }),
                    Some(explicit_session),
                ),
                &client(),
            )
            .await;
        assert!(matches!(clarification.reply, ServerReply::Success { .. }));

        let snapshot = telemetry.snapshot();
        assert_eq!(
            snapshot.route_decisions(RouteRule::Coordinator, TelemetryOutcome::Failed),
            1
        );
        assert_eq!(
            snapshot.route_decisions(RouteRule::Explicit, TelemetryOutcome::Failed),
            1
        );
        assert_eq!(
            snapshot.route_decisions(RouteRule::Clarification, TelemetryOutcome::Success),
            1
        );
        assert_eq!(snapshot.rejected_records(), 0);
    }

    #[tokio::test]
    async fn prompt_replays_durably_and_rejects_conflicting_request_reuse() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                ..FakeBehavior::default()
            },
        )
        .expect("application");
        let session_id = create(&application).await;
        let prompt = command(
            Command::SessionPrompt(PromptParams {
                text: "original".to_owned(),
                explicit_target: None,
            }),
            Some(session_id),
        );
        let first = application.dispatch(prompt.clone(), &client()).await;
        let ServerReply::Success {
            result: first_result,
            ..
        } = first.reply
        else {
            panic!("first prompt failed");
        };
        let events_after_first = application.history(session_id).expect("first history");

        let replay = application.dispatch(prompt.clone(), &client()).await;
        let ServerReply::Success {
            result: replay_result,
            ..
        } = replay.reply
        else {
            panic!("prompt replay failed");
        };
        assert_eq!(replay_result, first_result);
        assert_eq!(
            application.history(session_id).expect("replay history"),
            events_after_first
        );

        let conflict = application
            .dispatch(
                ClientCommand {
                    command: Command::SessionPrompt(PromptParams {
                        text: "changed".to_owned(),
                        explicit_target: None,
                    }),
                    ..prompt
                },
                &client(),
            )
            .await;
        let ServerReply::Failure { error, .. } = conflict.reply else {
            panic!("conflicting request was accepted");
        };
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(
            application.history(session_id).expect("conflict history"),
            events_after_first
        );
    }

    #[tokio::test]
    async fn cancel_deadline_produces_unknown_outcome() {
        let telemetry = Arc::new(BoundedTelemetry::default());
        let application = Application::in_memory_with_telemetry(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_secs(1),
                confirms_cancellation: false,
                cancellation_deadline: Duration::from_millis(5),
            },
            telemetry.clone(),
        )
        .expect("application");
        let session_id = create(&application).await;
        application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "hello".to_owned(),
                        explicit_target: None,
                    }),
                    Some(session_id),
                ),
                &client(),
            )
            .await;
        grant_pending(&application, session_id).await;
        application
            .dispatch(
                command(Command::SessionCancel(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(15)).await;
        let result = application
            .dispatch(
                command(Command::SessionGet(EmptyParams {}), Some(session_id)),
                &client(),
            )
            .await;
        let ServerReply::Success { result, .. } = result.reply else {
            panic!("get failed");
        };
        assert_eq!(result["state"], "outcome_unknown");
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.attempts(TelemetryOutcome::Timeout), 1);
        assert_eq!(snapshot.attempts(TelemetryOutcome::OutcomeUnknown), 1);
    }

    #[tokio::test]
    async fn records_failed_and_cancelled_attempt_paths() {
        let telemetry = Arc::new(BoundedTelemetry::default());
        let application = Application::in_memory_with_telemetry(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior {
                response_delay: Duration::from_mins(1),
                ..FakeBehavior::default()
            },
            telemetry.clone(),
        )
        .expect("application");
        let failed_session = create(&application).await;
        application
            .append(
                failed_session,
                None,
                EventKind::SessionFailed,
                terminal_payload("synthetic provider failure"),
                Some(Uuid::now_v7()),
                Some("paid-inference"),
            )
            .expect("failed event");

        let cancelled_session = create(&application).await;
        application
            .dispatch(
                command(
                    Command::SessionPrompt(PromptParams {
                        text: "cancel me".to_owned(),
                        explicit_target: None,
                    }),
                    Some(cancelled_session),
                ),
                &client(),
            )
            .await;
        application
            .dispatch(
                command(
                    Command::SessionCancel(EmptyParams {}),
                    Some(cancelled_session),
                ),
                &client(),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.attempts(TelemetryOutcome::Failed), 1);
        assert_eq!(snapshot.attempts(TelemetryOutcome::Cancelled), 1);
        assert_eq!(snapshot.rejected_records(), 0);
    }

    #[tokio::test]
    async fn rejects_commands_when_initialize_has_no_compatible_version() {
        let application = Application::in_memory(
            StartupConfiguration::safe_builtins().expect("startup"),
            FakeBehavior::default(),
        )
        .expect("application");
        let result = application
            .dispatch(
                command(
                    Command::Initialize(InitializeParams {
                        client_name: "test".to_owned(),
                        client_version: "1".to_owned(),
                        supported_protocols: vec!["workbench/2".to_owned()],
                    }),
                    None,
                ),
                &client(),
            )
            .await;
        assert!(matches!(result.reply, ServerReply::Failure { .. }));
    }
}
