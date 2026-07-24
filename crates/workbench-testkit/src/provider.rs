use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use futures_util::stream;
use workbench_core::{
    AttemptId, CoreError, FailureCategory,
    ports::{
        AuthenticationStatus, CancellationStatus, ProviderAdapter, ProviderCapabilities,
        ProviderCapability, ProviderFailure, ProviderOutput, ProviderPrompt, ProviderRegistry,
        ProviderSessionHandle, ProviderStream,
    },
    routing::RouteCandidate,
    value::{NonEmptyText, ProviderId},
};

use crate::DenyNetwork;

#[derive(Clone, Debug)]
pub enum SetupBehavior {
    Succeed(String),
    Fail(ProviderFailure),
}

#[derive(Clone, Debug)]
pub enum ResumeBehavior {
    Echo,
    Succeed(String),
    Fail(ProviderFailure),
}

#[derive(Clone, Debug)]
pub enum StreamBehavior {
    Emit(Vec<Result<ProviderOutput, ProviderFailure>>),
    FailSetup(ProviderFailure),
}

#[derive(Clone, Debug)]
pub enum CoordinatorBehavior {
    Candidate(RouteCandidate),
    Fail(CoreError),
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderCallCounts {
    pub capabilities: u64,
    pub authentication: u64,
    pub start: u64,
    pub resume: u64,
    pub prompt: u64,
    pub cancel: u64,
    pub classify: u64,
}

#[derive(Default)]
struct ProviderCounters {
    capabilities: AtomicU64,
    authentication: AtomicU64,
    start: AtomicU64,
    resume: AtomicU64,
    prompt: AtomicU64,
    cancel: AtomicU64,
    classify: AtomicU64,
}

#[derive(Clone)]
pub struct FakeProvider {
    state: Arc<FakeProviderState>,
}

struct FakeProviderState {
    capabilities: ProviderCapabilities,
    capabilities_error: Option<CoreError>,
    authentication_error: Option<CoreError>,
    start: SetupBehavior,
    resume: ResumeBehavior,
    stream: StreamBehavior,
    cancellation: Result<CancellationStatus, CoreError>,
    coordinator: CoordinatorBehavior,
    counters: ProviderCounters,
    prompts: Mutex<Vec<ProviderPrompt>>,
    network: DenyNetwork,
}

pub struct FakeProviderBuilder {
    capabilities: ProviderCapabilities,
    capabilities_error: Option<CoreError>,
    authentication_error: Option<CoreError>,
    start: SetupBehavior,
    resume: ResumeBehavior,
    stream: StreamBehavior,
    cancellation: Result<CancellationStatus, CoreError>,
    coordinator: CoordinatorBehavior,
    network: DenyNetwork,
}

impl FakeProvider {
    pub fn builder(adapter_id: ProviderId) -> FakeProviderBuilder {
        FakeProviderBuilder {
            capabilities: ProviderCapabilities {
                adapter_id,
                adapter_version: "1.0.0-test".to_owned(),
                protocol: "workbench-test/1".to_owned(),
                authentication: AuthenticationStatus::Available,
                capabilities: vec![
                    ProviderCapability::Streaming,
                    ProviderCapability::SessionResume,
                    ProviderCapability::Cancellation,
                ],
                context_window_tokens: Some(8_192),
            },
            capabilities_error: None,
            authentication_error: None,
            start: SetupBehavior::Succeed("fake-session".to_owned()),
            resume: ResumeBehavior::Echo,
            stream: StreamBehavior::Emit(vec![
                Ok(ProviderOutput::Acknowledged {
                    provider_request_id: Some("fake-request".to_owned()),
                }),
                Ok(ProviderOutput::Content {
                    event_type: "text_delta".to_owned(),
                    content: NonEmptyText::parse("deterministic output").expect("static text"),
                }),
                Ok(ProviderOutput::Tool {
                    event_type: "tool_result".to_owned(),
                    content: NonEmptyText::parse("deterministic tool output").expect("static text"),
                }),
                Ok(ProviderOutput::Completed {
                    summary: "completed".to_owned(),
                }),
            ]),
            cancellation: Ok(CancellationStatus::Confirmed),
            coordinator: CoordinatorBehavior::Unsupported,
            network: DenyNetwork::default(),
        }
    }

    #[must_use]
    pub fn call_counts(&self) -> ProviderCallCounts {
        let counters = &self.state.counters;
        ProviderCallCounts {
            capabilities: counters.capabilities.load(Ordering::Relaxed),
            authentication: counters.authentication.load(Ordering::Relaxed),
            start: counters.start.load(Ordering::Relaxed),
            resume: counters.resume.load(Ordering::Relaxed),
            prompt: counters.prompt.load(Ordering::Relaxed),
            cancel: counters.cancel.load(Ordering::Relaxed),
            classify: counters.classify.load(Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn prompts(&self) -> Vec<ProviderPrompt> {
        self.state
            .prompts
            .lock()
            .expect("fake provider prompt mutex poisoned")
            .clone()
    }

    #[must_use]
    pub fn network_guard(&self) -> DenyNetwork {
        self.state.network.clone()
    }
}

impl FakeProviderBuilder {
    #[must_use]
    pub fn capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    #[must_use]
    pub fn capabilities_error(mut self, error: CoreError) -> Self {
        self.capabilities_error = Some(error);
        self
    }

    #[must_use]
    pub fn authentication_error(mut self, error: CoreError) -> Self {
        self.authentication_error = Some(error);
        self
    }

    #[must_use]
    pub fn start(mut self, behavior: SetupBehavior) -> Self {
        self.start = behavior;
        self
    }

    #[must_use]
    pub fn resume(mut self, behavior: ResumeBehavior) -> Self {
        self.resume = behavior;
        self
    }

    #[must_use]
    pub fn stream(mut self, behavior: StreamBehavior) -> Self {
        self.stream = behavior;
        self
    }

    #[must_use]
    pub fn cancellation(mut self, result: Result<CancellationStatus, CoreError>) -> Self {
        self.cancellation = result;
        self
    }

    #[must_use]
    pub fn coordinator(mut self, behavior: CoordinatorBehavior) -> Self {
        self.coordinator = behavior;
        self
    }

    #[must_use]
    pub fn network_guard(mut self, guard: DenyNetwork) -> Self {
        self.network = guard;
        self
    }

    #[must_use]
    pub fn build(self) -> FakeProvider {
        FakeProvider {
            state: Arc::new(FakeProviderState {
                capabilities: self.capabilities,
                capabilities_error: self.capabilities_error,
                authentication_error: self.authentication_error,
                start: self.start,
                resume: self.resume,
                stream: self.stream,
                cancellation: self.cancellation,
                coordinator: self.coordinator,
                counters: ProviderCounters::default(),
                prompts: Mutex::default(),
                network: self.network,
            }),
        }
    }
}

#[async_trait]
impl ProviderAdapter for FakeProvider {
    async fn capabilities(&self) -> Result<ProviderCapabilities, CoreError> {
        self.state
            .counters
            .capabilities
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .capabilities_error
            .clone()
            .map_or_else(|| Ok(self.state.capabilities.clone()), Err)
    }

    async fn authentication_status(&self) -> Result<AuthenticationStatus, CoreError> {
        self.state
            .counters
            .authentication
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .authentication_error
            .clone()
            .map_or(Ok(self.state.capabilities.authentication), Err)
    }

    async fn start_session(&self) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.state.counters.start.fetch_add(1, Ordering::Relaxed);
        setup_result(&self.state.start)
    }

    async fn resume_session(
        &self,
        opaque_handle: &str,
    ) -> Result<ProviderSessionHandle, ProviderFailure> {
        self.state.counters.resume.fetch_add(1, Ordering::Relaxed);
        match &self.state.resume {
            ResumeBehavior::Echo => {
                ProviderSessionHandle::new(opaque_handle).map_err(|error| core_failure(&error))
            }
            ResumeBehavior::Succeed(handle) => {
                ProviderSessionHandle::new(handle.clone()).map_err(|error| core_failure(&error))
            }
            ResumeBehavior::Fail(error) => Err(error.clone()),
        }
    }

    async fn prompt_stream(
        &self,
        _handle: &ProviderSessionHandle,
        prompt: ProviderPrompt,
    ) -> Result<ProviderStream, ProviderFailure> {
        self.state.counters.prompt.fetch_add(1, Ordering::Relaxed);
        self.state
            .prompts
            .lock()
            .expect("fake provider prompt mutex poisoned")
            .push(prompt);
        match &self.state.stream {
            StreamBehavior::Emit(items) => Ok(Box::pin(stream::iter(items.clone()))),
            StreamBehavior::FailSetup(error) => Err(error.clone()),
        }
    }

    async fn cancel(
        &self,
        _handle: &ProviderSessionHandle,
        _attempt_id: AttemptId,
    ) -> Result<CancellationStatus, CoreError> {
        self.state.counters.cancel.fetch_add(1, Ordering::Relaxed);
        self.state.cancellation.clone()
    }

    async fn classify(&self, _input: NonEmptyText) -> Result<RouteCandidate, CoreError> {
        self.state.counters.classify.fetch_add(1, Ordering::Relaxed);
        match &self.state.coordinator {
            CoordinatorBehavior::Candidate(candidate) => Ok(candidate.clone()),
            CoordinatorBehavior::Fail(error) => Err(error.clone()),
            CoordinatorBehavior::Unsupported => Err(CoreError::new(
                FailureCategory::CapabilityUnavailable,
                "fake coordinator classification is not configured",
            )),
        }
    }
}

fn setup_result(behavior: &SetupBehavior) -> Result<ProviderSessionHandle, ProviderFailure> {
    match behavior {
        SetupBehavior::Succeed(handle) => {
            ProviderSessionHandle::new(handle.clone()).map_err(|error| core_failure(&error))
        }
        SetupBehavior::Fail(error) => Err(error.clone()),
    }
}

fn core_failure(error: &CoreError) -> ProviderFailure {
    ProviderFailure {
        category: error.category(),
        user_safe_message: error.message().to_owned(),
        definite: true,
    }
}

#[derive(Clone, Default)]
pub struct FakeProviderRegistry {
    adapters: Arc<Mutex<BTreeMap<ProviderId, Arc<dyn ProviderAdapter>>>>,
}

impl FakeProviderRegistry {
    pub fn register(
        &self,
        provider: ProviderId,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters
            .lock()
            .expect("fake registry mutex poisoned")
            .insert(provider, adapter)
    }

    pub fn remove(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters
            .lock()
            .expect("fake registry mutex poisoned")
            .remove(provider)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.adapters
            .lock()
            .expect("fake registry mutex poisoned")
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ProviderRegistry for FakeProviderRegistry {
    fn adapter(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters
            .lock()
            .expect("fake registry mutex poisoned")
            .get(provider)
            .cloned()
    }
}
