use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::stream;
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use uuid::Uuid;
use workbench_core::{
    AttemptId, CoreError, FailureCategory,
    ports::{
        AuthenticationStatus, CancellationStatus, ProviderAdapter, ProviderCapabilities,
        ProviderCapability, ProviderFailure, ProviderOutput, ProviderPrompt, ProviderSessionHandle,
        ProviderStream,
    },
    value::ProviderId,
};

use crate::{
    DEFAULT_BASE_URL, OPENROUTER_CHAT_COMPLETIONS_PROTOCOL, OpenRouterError, OpenRouterErrorKind,
    budget::{
        CostPolicyConfig, SessionCostLedger, deny_error, estimate_attempt_usd_micros,
        evaluate_budget,
    },
    credential::{SecretSource, require_secret},
    transport::{OpenRouterTransport, decode_body},
};

const MAX_ADAPTER_VERSION_BYTES: usize = 256;

struct LocalSession {
    setup: AsyncMutex<()>,
    active: Mutex<Option<ActiveAttempt>>,
    pending_cancellations: Mutex<HashSet<AttemptId>>,
}

#[derive(Clone)]
struct ActiveAttempt {
    attempt_id: AttemptId,
    cancel: Arc<AtomicBool>,
    respond: Arc<Mutex<Option<oneshot::Sender<CancellationStatus>>>>,
}

/// Construction inputs for [`OpenRouterProviderAdapter::connect`].
pub struct OpenRouterConnect {
    pub adapter_id: ProviderId,
    pub adapter_version: String,
    pub credential_ref: String,
    pub secrets: Arc<dyn SecretSource>,
    pub transport: OpenRouterTransport,
    pub ledger: SessionCostLedger,
    pub policy: CostPolicyConfig,
    pub zero_data_retention: bool,
    pub cancellation_deadline: Duration,
    pub require_secret_at_connect: bool,
}

/// Provider-port implementation for OpenRouter Chat Completions.
pub struct OpenRouterProviderAdapter {
    adapter_id: ProviderId,
    adapter_version: String,
    credential_ref: String,
    secrets: Arc<dyn SecretSource>,
    transport: OpenRouterTransport,
    ledger: SessionCostLedger,
    policy: CostPolicyConfig,
    zero_data_retention: bool,
    sessions: Arc<RwLock<HashMap<String, Arc<LocalSession>>>>,
    cancellation_deadline: Duration,
    shutting_down: AtomicBool,
}

impl OpenRouterProviderAdapter {
    /// Connects an offline or configured OpenRouter adapter after secret probe.
    ///
    /// # Errors
    ///
    /// Returns when configuration is invalid. Missing secrets leave auth
    /// unavailable but still allow constructing the adapter for fail-closed
    /// prompt paths when `require_secret_at_connect` is false.
    pub fn connect(input: OpenRouterConnect) -> Result<Self, CoreError> {
        if input.cancellation_deadline.is_zero()
            || input.adapter_version.is_empty()
            || input.adapter_version.len() > MAX_ADAPTER_VERSION_BYTES
            || input.credential_ref.is_empty()
            || input.policy.max_session_usd_micros == 0
        {
            return Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "OpenRouter adapter configuration is invalid",
            ));
        }
        if input.require_secret_at_connect {
            require_secret(input.secrets.as_ref(), &input.credential_ref)
                .map_err(CoreError::from)?;
        }
        Ok(Self {
            adapter_id: input.adapter_id,
            adapter_version: input.adapter_version,
            credential_ref: input.credential_ref,
            secrets: input.secrets,
            transport: input.transport,
            ledger: input.ledger,
            policy: input.policy,
            zero_data_retention: input.zero_data_retention,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cancellation_deadline: input.cancellation_deadline,
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Builds a transport for the configured base URL (fake when `fake://`).
    #[must_use]
    pub fn transport_for_base_url(base_url: Option<&str>) -> OpenRouterTransport {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL);
        OpenRouterTransport::offline(base)
    }

    #[must_use]
    pub fn ledger(&self) -> &SessionCostLedger {
        &self.ledger
    }

    #[must_use]
    pub fn transport(&self) -> &OpenRouterTransport {
        &self.transport
    }

    #[must_use]
    pub const fn zero_data_retention(&self) -> bool {
        self.zero_data_retention
    }

    /// Stops new work and clears active sessions.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        if let Ok(mut sessions) = self.sessions.write() {
            sessions.clear();
        }
    }

    fn auth_status(&self) -> AuthenticationStatus {
        match require_secret(self.secrets.as_ref(), &self.credential_ref) {
            Ok(_) => AuthenticationStatus::Available,
            Err(_) => AuthenticationStatus::Unavailable,
        }
    }

    fn ensure_active(&self) -> Result<(), ProviderFailure> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::ShuttingDown,
                "OpenRouter adapter is shutting down",
            )
            .into_provider_failure(true));
        }
        Ok(())
    }
}

#[async_trait]
impl ProviderAdapter for OpenRouterProviderAdapter {
    async fn capabilities(&self) -> Result<ProviderCapabilities, CoreError> {
        Ok(ProviderCapabilities {
            adapter_id: self.adapter_id.clone(),
            adapter_version: self.adapter_version.clone(),
            protocol: OPENROUTER_CHAT_COMPLETIONS_PROTOCOL.to_owned(),
            authentication: self.auth_status(),
            capabilities: vec![ProviderCapability::Streaming, ProviderCapability::Cancellation],
            context_window_tokens: Some(128_000),
        })
    }

    async fn authentication_status(&self) -> Result<AuthenticationStatus, CoreError> {
        Ok(self.auth_status())
    }

    async fn start_session(&self) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.ensure_active()?;
        let handle = Uuid::now_v7().to_string();
        let session = Arc::new(LocalSession {
            setup: AsyncMutex::new(()),
            active: Mutex::new(None),
            pending_cancellations: Mutex::new(HashSet::new()),
        });
        self.sessions
            .write()
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "session map unavailable")
                    .into_provider_failure(true)
            })?
            .insert(handle.clone(), session);
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
        Err(ProviderFailure {
            category: FailureCategory::CapabilityUnavailable,
            user_safe_message: "OpenRouter adapter does not support session resume".to_owned(),
            definite: true,
        })
    }

    async fn prompt_stream(
        &self,
        handle: &ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) -> Result<ProviderStream, ProviderFailure> {
        self.ensure_active()?;
        let session = self
            .sessions
            .read()
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "session map unavailable")
                    .into_provider_failure(true)
            })?
            .get(handle.expose_to_adapter())
            .cloned()
            .ok_or_else(|| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "unknown provider session")
                    .into_provider_failure(true)
            })?;

        let _setup = session.setup.lock().await;
        if session
            .active
            .lock()
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "session lock unavailable")
                    .into_provider_failure(true)
            })?
            .is_some()
        {
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::Unavailable,
                "OpenRouter session already has an active attempt",
            )
            .into_provider_failure(true));
        }

        let secret = require_secret(self.secrets.as_ref(), &self.credential_ref)
            .map_err(|error| error.into_provider_failure(true))?;

        let estimate = estimate_attempt_usd_micros(self.policy);
        let decision = evaluate_budget(self.policy, &self.ledger, estimate);
        deny_error(decision).map_err(|error| error.into_provider_failure(true))?;

        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = session.active.lock().map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "session lock unavailable")
                    .into_provider_failure(true)
            })?;
            *active = Some(ActiveAttempt {
                attempt_id: prompt.attempt_id,
                cancel: Arc::clone(&cancel),
                respond: Arc::new(Mutex::new(None)),
            });
        }

        if session
            .pending_cancellations
            .lock()
            .map(|pending| pending.contains(&prompt.attempt_id))
            .unwrap_or(false)
        {
            clear_active(&session, prompt.attempt_id);
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::Cancelled,
                "OpenRouter attempt was cancelled before dispatch",
            )
            .into_provider_failure(true));
        }

        let transport_result =
            self.transport
                .chat_completion(&secret, &prompt.runtime_model, prompt.content.as_str());
        drop(secret);

        let result = match transport_result {
            Ok(result) => result,
            Err(error) => {
                clear_active(&session, prompt.attempt_id);
                // Failures before stream emission are definite pre-dispatch when
                // transport never started; credential/budget already handled.
                let definite = matches!(
                    error.kind(),
                    OpenRouterErrorKind::CredentialMissing
                        | OpenRouterErrorKind::CredentialEmpty
                        | OpenRouterErrorKind::BudgetExceeded
                        | OpenRouterErrorKind::InvalidConfig
                );
                return Err(error.into_provider_failure(definite));
            }
        };

        if cancel.load(Ordering::Acquire) {
            clear_active(&session, prompt.attempt_id);
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::Cancelled,
                "OpenRouter attempt was cancelled",
            )
            .into_provider_failure(true));
        }

        let decoded = match decode_body(&result.body) {
            Ok(decoded) => decoded,
            Err(error) => {
                clear_active(&session, prompt.attempt_id);
                // Body was received → not definite success; treat as uncertain if
                // partial content may have been produced server-side.
                return Err(error.into_provider_failure(false));
            }
        };

        let (mut outputs, completed) = decoded;
        if !completed && outputs.is_empty() {
            clear_active(&session, prompt.attempt_id);
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::OutcomeUnknown,
                "OpenRouter stream ended without a definite completion",
            )
            .into_provider_failure(false));
        }

        if completed {
            let spend = result.usage.cost_usd_micros.max(1);
            self.ledger.record_spend(spend);
            outputs.push(ProviderOutput::Completed {
                summary: "openrouter completed".to_owned(),
            });
        } else {
            clear_active(&session, prompt.attempt_id);
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::OutcomeUnknown,
                "OpenRouter stream ended without a terminal marker",
            )
            .into_provider_failure(false));
        }

        clear_active(&session, prompt.attempt_id);

        let stream = stream::iter(outputs.into_iter().map(Ok::<_, ProviderFailure>));
        Ok(Box::pin(stream))
    }

    async fn cancel(
        &self,
        handle: &ProviderSessionHandle,
        attempt_id: AttemptId,
    ) -> Result<CancellationStatus, CoreError> {
        let session = self
            .sessions
            .read()
            .map_err(|_| {
                CoreError::new(
                    FailureCategory::ProviderUnavailable,
                    "session map unavailable",
                )
            })?
            .get(handle.expose_to_adapter())
            .cloned();
        let Some(session) = session else {
            return Ok(CancellationStatus::Unconfirmed);
        };
        if let Ok(mut pending) = session.pending_cancellations.lock() {
            pending.insert(attempt_id);
        }
        let active = session
            .active
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .filter(|active| active.attempt_id == attempt_id);
        if let Some(active) = active {
            active.cancel.store(true, Ordering::Release);
            // Offline transport is synchronous; cancellation before stream
            // handoff is confirmed.
            return Ok(CancellationStatus::Confirmed);
        }
        let _ = self.cancellation_deadline;
        Ok(CancellationStatus::Unconfirmed)
    }
}

fn clear_active(session: &LocalSession, attempt_id: AttemptId) {
    if let Ok(mut active) = session.active.lock()
        && active
            .as_ref()
            .is_some_and(|current| current.attempt_id == attempt_id)
    {
        *active = None;
    }
    if let Ok(mut pending) = session.pending_cancellations.lock() {
        pending.remove(&attempt_id);
    }
}

