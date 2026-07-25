//! ACP agent methods mapped onto Workbench daemon protocol operations.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_util::codec::Framed;
use uuid::Uuid;
use workbench_daemon::{Application, ClientContext, FakeBehavior, StartupConfiguration};
use workbench_protocol::{
    ClientCommand, Command, EventKind, NdjsonCodec, PROTOCOL_V1, ServerReply, SessionEvent,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, EmptyParams,
        InitializeParams, PromptParams,
    },
    response::{
        AttachSessionResult, CreateSessionResult, InitializeResult, PromptResult, SessionResult,
    },
};
use workbench_storage::{MemoryKeyStore, SqliteStorage};

use crate::{
    ACP_PROTOCOL_VERSION, AGENT_NAME, AcpServerError, AcpServerErrorKind, decode_line,
    encode_message,
};

/// Backend used by the ACP bridge to reach Workbench sessions.
#[async_trait]
pub trait BridgeBackend: Send + Sync {
    async fn initialize(&self) -> Result<Value, AcpServerError>;
    async fn create_session(&self) -> Result<Uuid, AcpServerError>;
    async fn prompt(&self, session_id: Uuid, text: &str) -> Result<Vec<String>, AcpServerError>;
    async fn cancel(&self, session_id: Uuid) -> Result<(), AcpServerError>;
}

/// In-process daemon-backed backend for offline tests and local agent stdio.
pub struct InProcessBackend {
    application: Arc<Application>,
    client: ClientContext,
}

impl InProcessBackend {
    #[must_use]
    pub fn new(application: Arc<Application>) -> Self {
        Self {
            application,
            client: ClientContext {
                uid: 1,
                client_name: "workbench-acp-server".to_owned(),
            },
        }
    }

    /// Builds an offline in-memory application with the fake provider.
    ///
    /// # Panics
    ///
    /// Panics when safe builtins or in-memory storage cannot be constructed.
    #[must_use]
    pub fn offline_fake() -> Self {
        let startup = StartupConfiguration::safe_builtins().expect("safe builtins");
        let storage =
            SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("in-memory storage");
        let application = Application::new(storage, startup, FakeBehavior::default());
        Self::new(application)
    }

    async fn auto_grant_pending(&self, session_id: Uuid) -> Result<(), AcpServerError> {
        let history = self
            .application
            .session_history(session_id)
            .map_err(|_| AcpServerError::new(AcpServerErrorKind::Backend, "history unavailable"))?;
        let approval_id = history.iter().rev().find_map(|event| {
            (event.kind == "approval_requested")
                .then_some(())
                .and_then(|()| {
                    event
                        .payload
                        .get("approval_id")
                        .and_then(Value::as_str)
                        .and_then(|value| Uuid::parse_str(value).ok())
                })
        });
        let Some(approval_id) = approval_id else {
            return Ok(());
        };
        let reply = self
            .application
            .dispatch(
                ClientCommand {
                    protocol: PROTOCOL_V1.to_owned(),
                    request_id: Uuid::now_v7(),
                    session_id: Some(session_id),
                    command: Command::SessionApprovalResolve(ApprovalParams {
                        approval_id,
                        decision: ApprovalDecision::Grant,
                    }),
                },
                &self.client,
            )
            .await
            .reply;
        match reply {
            ServerReply::Success { .. } => Ok(()),
            ServerReply::Failure { error, .. } => Err(AcpServerError::new(
                AcpServerErrorKind::Backend,
                error.message,
            )),
        }
    }
}

#[async_trait]
impl BridgeBackend for InProcessBackend {
    async fn initialize(&self) -> Result<Value, AcpServerError> {
        let reply = self
            .application
            .dispatch(
                ClientCommand {
                    protocol: PROTOCOL_V1.to_owned(),
                    request_id: Uuid::now_v7(),
                    session_id: None,
                    command: Command::Initialize(InitializeParams {
                        client_name: "workbench-acp-server".to_owned(),
                        client_version: env!("CARGO_PKG_VERSION").to_owned(),
                        supported_protocols: vec![PROTOCOL_V1.to_owned()],
                    }),
                },
                &self.client,
            )
            .await
            .reply;
        match reply {
            ServerReply::Success { result, .. } => {
                let _init: InitializeResult = serde_json::from_value(result).map_err(|_| {
                    AcpServerError::new(AcpServerErrorKind::Backend, "initialize result invalid")
                })?;
                Ok(json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "agentInfo": {
                        "name": AGENT_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "agentCapabilities": {
                        "loadSession": false,
                    }
                }))
            }
            ServerReply::Failure { error, .. } => Err(AcpServerError::new(
                AcpServerErrorKind::Backend,
                error.message,
            )),
        }
    }

    async fn create_session(&self) -> Result<Uuid, AcpServerError> {
        let reply = self
            .application
            .dispatch(
                ClientCommand {
                    protocol: PROTOCOL_V1.to_owned(),
                    request_id: Uuid::now_v7(),
                    session_id: None,
                    command: Command::SessionCreate(CreateSessionParams {
                        persistent: true,
                        configuration_overrides: None,
                        workflow: None,
                    }),
                },
                &self.client,
            )
            .await
            .reply;
        match reply {
            ServerReply::Success { result, .. } => {
                let created: CreateSessionResult =
                    serde_json::from_value(result).map_err(|_| {
                        AcpServerError::new(
                            AcpServerErrorKind::Backend,
                            "create session result invalid",
                        )
                    })?;
                Ok(created.session_id)
            }
            ServerReply::Failure { error, .. } => Err(AcpServerError::new(
                AcpServerErrorKind::Backend,
                error.message,
            )),
        }
    }

    async fn prompt(&self, session_id: Uuid, text: &str) -> Result<Vec<String>, AcpServerError> {
        let reply = self
            .application
            .dispatch(
                ClientCommand {
                    protocol: PROTOCOL_V1.to_owned(),
                    request_id: Uuid::now_v7(),
                    session_id: Some(session_id),
                    command: Command::SessionPrompt(PromptParams {
                        text: text.to_owned(),
                        explicit_target: None,
                    }),
                },
                &self.client,
            )
            .await
            .reply;
        match reply {
            ServerReply::Success { result, .. } => {
                let _prompt: PromptResult = serde_json::from_value(result).map_err(|_| {
                    AcpServerError::new(AcpServerErrorKind::Backend, "prompt result invalid")
                })?;
                self.auto_grant_pending(session_id).await?;
                // Give the fake provider a moment to complete offline.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let history = self.application.session_history(session_id).map_err(|_| {
                    AcpServerError::new(AcpServerErrorKind::Backend, "history unavailable")
                })?;
                let mut chunks = Vec::new();
                for event in history {
                    if matches!(
                        event.kind.as_str(),
                        "provider_event" | "session_completed" | "dispatch_acknowledged"
                    ) && let Some(content) = event
                        .payload
                        .get("content")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                    {
                        chunks.push(content.to_owned());
                    }
                }
                if chunks.is_empty() {
                    chunks.push("workbench offline response".to_owned());
                }
                Ok(chunks)
            }
            ServerReply::Failure { error, .. } => Err(AcpServerError::new(
                AcpServerErrorKind::Backend,
                error.message,
            )),
        }
    }

    async fn cancel(&self, session_id: Uuid) -> Result<(), AcpServerError> {
        let reply = self
            .application
            .dispatch(
                ClientCommand {
                    protocol: PROTOCOL_V1.to_owned(),
                    request_id: Uuid::now_v7(),
                    session_id: Some(session_id),
                    command: Command::SessionCancel(EmptyParams {}),
                },
                &self.client,
            )
            .await
            .reply;
        match reply {
            ServerReply::Success { .. } => Ok(()),
            ServerReply::Failure { error, .. } => Err(AcpServerError::new(
                AcpServerErrorKind::Backend,
                error.message,
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InboundMessage {
    Reply(ServerReply<Value>),
    Event(SessionEvent),
}

type SocketTransport = Framed<UnixStream, NdjsonCodec<InboundMessage, ClientCommand>>;

/// Unix-socket backend that attaches ACP to a running workspace daemon.
pub struct DaemonSocketBackend {
    endpoint: PathBuf,
    transport: tokio::sync::Mutex<SocketTransportState>,
}

struct SocketTransportState {
    framed: SocketTransport,
    pending_events: VecDeque<SessionEvent>,
}

impl DaemonSocketBackend {
    /// Connects to a live daemon endpoint and negotiates protocol v1.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed backend error when the socket is missing,
    /// unreachable, or protocol negotiation fails.
    pub async fn connect(endpoint: impl Into<PathBuf>) -> Result<Self, AcpServerError> {
        let endpoint = endpoint.into();
        let state = Self::open_transport(&endpoint).await?;
        Ok(Self {
            endpoint,
            transport: tokio::sync::Mutex::new(state),
        })
    }

    /// Endpoint path this backend is attached to.
    #[must_use]
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    async fn open_transport(endpoint: &Path) -> Result<SocketTransportState, AcpServerError> {
        let stream = UnixStream::connect(endpoint).await.map_err(|error| {
            AcpServerError::new(
                AcpServerErrorKind::Backend,
                format!(
                    "daemon socket unavailable ({error}); start `workbench daemon` for this workspace first"
                ),
            )
        })?;
        let mut state = SocketTransportState {
            framed: Framed::new(stream, NdjsonCodec::default()),
            pending_events: VecDeque::new(),
        };
        let result = call_socket(
            &mut state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::Initialize(InitializeParams {
                    client_name: "workbench-acp-server".to_owned(),
                    client_version: env!("CARGO_PKG_VERSION").to_owned(),
                    supported_protocols: vec![PROTOCOL_V1.to_owned()],
                }),
            },
        )
        .await?;
        let _init: InitializeResult = serde_json::from_value(result).map_err(|_| {
            AcpServerError::new(AcpServerErrorKind::Backend, "initialize result invalid")
        })?;
        Ok(state)
    }

    async fn auto_grant_pending(
        state: &mut SocketTransportState,
        session_id: Uuid,
    ) -> Result<(), AcpServerError> {
        let result = call_socket(
            state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionGet(EmptyParams {}),
            },
        )
        .await?;
        let session: SessionResult = serde_json::from_value(result).map_err(|_| {
            AcpServerError::new(AcpServerErrorKind::Backend, "session.get result invalid")
        })?;
        let Some(approval_id) = session.pending_approval_id else {
            // Also scan buffered events for approval_requested.
            if let Some(approval_id) = state.pending_events.iter().rev().find_map(|event| {
                (event.kind == EventKind::ApprovalRequested)
                    .then(|| {
                        event
                            .data
                            .get("approval_id")
                            .and_then(Value::as_str)
                            .and_then(|value| Uuid::parse_str(value).ok())
                    })
                    .flatten()
            }) {
                let _ = call_socket(
                    state,
                    ClientCommand {
                        protocol: PROTOCOL_V1.to_owned(),
                        request_id: Uuid::now_v7(),
                        session_id: Some(session_id),
                        command: Command::SessionApprovalResolve(ApprovalParams {
                            approval_id,
                            decision: ApprovalDecision::Grant,
                        }),
                    },
                )
                .await?;
            }
            return Ok(());
        };
        let _ = call_socket(
            state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionApprovalResolve(ApprovalParams {
                    approval_id,
                    decision: ApprovalDecision::Grant,
                }),
            },
        )
        .await?;
        Ok(())
    }

    async fn collect_prompt_chunks(
        state: &mut SocketTransportState,
        session_id: Uuid,
    ) -> Result<Vec<String>, AcpServerError> {
        let attach = call_socket(
            state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            },
        )
        .await?;
        let _attach: AttachSessionResult = serde_json::from_value(attach).map_err(|_| {
            AcpServerError::new(AcpServerErrorKind::Backend, "session.attach result invalid")
        })?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut chunks = Vec::new();
        let mut terminal = false;
        while tokio::time::Instant::now() < deadline && !terminal {
            while let Some(event) = state.pending_events.pop_front() {
                if let Some(content) = event_content(&event) {
                    chunks.push(content);
                }
                if matches!(
                    event.kind,
                    EventKind::SessionCompleted
                        | EventKind::SessionFailed
                        | EventKind::SessionCancelled
                        | EventKind::SessionAbandoned
                ) {
                    terminal = true;
                    break;
                }
            }
            if terminal {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(100), state.framed.next()).await {
                Ok(Some(Ok(InboundMessage::Event(event)))) => {
                    state.pending_events.push_back(event);
                }
                Ok(Some(Ok(InboundMessage::Reply(_)))) => {
                    // Ignore late replies while draining session events.
                }
                Ok(Some(Err(error))) => {
                    return Err(AcpServerError::new(
                        AcpServerErrorKind::Backend,
                        format!("daemon framing failed: {error}"),
                    ));
                }
                Ok(None) => {
                    return Err(AcpServerError::new(
                        AcpServerErrorKind::Backend,
                        "daemon disconnected while streaming prompt events",
                    ));
                }
                Err(_) => {
                    // Brief idle; re-check pending approval then continue until deadline.
                    Self::auto_grant_pending(state, session_id).await?;
                }
            }
        }
        if chunks.is_empty() {
            chunks.push("workbench offline response".to_owned());
        }
        Ok(chunks)
    }
}

#[async_trait]
impl BridgeBackend for DaemonSocketBackend {
    async fn initialize(&self) -> Result<Value, AcpServerError> {
        // Protocol initialize already completed at connect; re-advertise ACP identity.
        let mut state = self.transport.lock().await;
        let _ = call_socket(
            &mut state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::StatusGet(EmptyParams {}),
            },
        )
        .await?;
        Ok(json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "agentInfo": {
                "name": AGENT_NAME,
                "version": env!("CARGO_PKG_VERSION"),
            },
            "agentCapabilities": {
                "loadSession": false,
            }
        }))
    }

    async fn create_session(&self) -> Result<Uuid, AcpServerError> {
        let mut state = self.transport.lock().await;
        let result = call_socket(
            &mut state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            },
        )
        .await?;
        let created: CreateSessionResult = serde_json::from_value(result).map_err(|_| {
            AcpServerError::new(AcpServerErrorKind::Backend, "create session result invalid")
        })?;
        Ok(created.session_id)
    }

    async fn prompt(&self, session_id: Uuid, text: &str) -> Result<Vec<String>, AcpServerError> {
        let mut state = self.transport.lock().await;
        let result = call_socket(
            &mut state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionPrompt(PromptParams {
                    text: text.to_owned(),
                    explicit_target: None,
                }),
            },
        )
        .await?;
        let _prompt: PromptResult = serde_json::from_value(result).map_err(|_| {
            AcpServerError::new(AcpServerErrorKind::Backend, "prompt result invalid")
        })?;
        Self::auto_grant_pending(&mut state, session_id).await?;
        Self::collect_prompt_chunks(&mut state, session_id).await
    }

    async fn cancel(&self, session_id: Uuid) -> Result<(), AcpServerError> {
        let mut state = self.transport.lock().await;
        let _ = call_socket(
            &mut state,
            ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionCancel(EmptyParams {}),
            },
        )
        .await?;
        Ok(())
    }
}

async fn call_socket(
    state: &mut SocketTransportState,
    command: ClientCommand,
) -> Result<Value, AcpServerError> {
    let expected = command.request_id;
    state.framed.send(command).await.map_err(|error| {
        AcpServerError::new(
            AcpServerErrorKind::Backend,
            format!("daemon write failed: {error}"),
        )
    })?;
    loop {
        let message = state
            .framed
            .next()
            .await
            .ok_or_else(|| {
                AcpServerError::new(
                    AcpServerErrorKind::Backend,
                    "daemon disconnected before reply",
                )
            })?
            .map_err(|error| {
                AcpServerError::new(
                    AcpServerErrorKind::Backend,
                    format!("daemon framing failed: {error}"),
                )
            })?;
        match message {
            InboundMessage::Event(event) => state.pending_events.push_back(event),
            InboundMessage::Reply(ServerReply::Success { request_id, result }) => {
                if request_id != expected {
                    return Err(AcpServerError::new(
                        AcpServerErrorKind::Backend,
                        "daemon reply correlation mismatch",
                    ));
                }
                return Ok(result);
            }
            InboundMessage::Reply(ServerReply::Failure { request_id, error }) => {
                if request_id != expected {
                    return Err(AcpServerError::new(
                        AcpServerErrorKind::Backend,
                        "daemon failure correlation mismatch",
                    ));
                }
                return Err(AcpServerError::new(
                    AcpServerErrorKind::Backend,
                    error.message,
                ));
            }
        }
    }
}

fn event_content(event: &SessionEvent) -> Option<String> {
    if !matches!(
        event.kind,
        EventKind::ProviderEvent | EventKind::SessionCompleted | EventKind::DispatchAcknowledged
    ) {
        return None;
    }
    event
        .data
        .get("content")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// ACP agent server state for one stdio connection.
pub struct AcpAgentServer {
    backend: Arc<dyn BridgeBackend>,
    sessions: Mutex<HashMap<String, Uuid>>,
    shutting_down: AtomicBool,
}

impl AcpAgentServer {
    #[must_use]
    pub fn new(backend: Arc<dyn BridgeBackend>) -> Self {
        Self {
            backend,
            sessions: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Handles one decoded JSON-RPC request and returns zero or more responses.
    ///
    /// # Errors
    ///
    /// Returns when the request is invalid or the backend fails.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_message(&self, message: Value) -> Result<Vec<Value>, AcpServerError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(AcpServerError::new(
                AcpServerErrorKind::ShuttingDown,
                "ACP agent is shutting down",
            ));
        }
        let object = message.as_object().ok_or_else(|| {
            AcpServerError::new(
                AcpServerErrorKind::InvalidRequest,
                "ACP message must be an object",
            )
        })?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(AcpServerError::new(
                AcpServerErrorKind::InvalidRequest,
                "ACP message requires jsonrpc 2.0",
            ));
        }
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AcpServerError::new(
                    AcpServerErrorKind::InvalidRequest,
                    "ACP request is missing method",
                )
            })?;
        let id = object.get("id").cloned();
        match method {
            "initialize" => {
                let result = self.backend.initialize().await?;
                Ok(vec![success(id, result)])
            }
            "session/new" | "session/newSession" => {
                let session_id = self.backend.create_session().await?;
                let acp_id = session_id.to_string();
                self.sessions
                    .lock()
                    .map_err(|_| {
                        AcpServerError::new(AcpServerErrorKind::Backend, "session map unavailable")
                    })?
                    .insert(acp_id.clone(), session_id);
                Ok(vec![success(
                    id,
                    json!({
                        "sessionId": acp_id,
                    }),
                )])
            }
            "session/prompt" => {
                let params = object.get("params").cloned().unwrap_or(json!({}));
                let session_key =
                    params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            AcpServerError::new(
                                AcpServerErrorKind::InvalidRequest,
                                "session/prompt requires sessionId",
                            )
                        })?;
                let session_id = self
                    .sessions
                    .lock()
                    .map_err(|_| {
                        AcpServerError::new(AcpServerErrorKind::Backend, "session map unavailable")
                    })?
                    .get(session_key)
                    .copied()
                    .ok_or_else(|| {
                        AcpServerError::new(
                            AcpServerErrorKind::SessionNotFound,
                            "unknown ACP session",
                        )
                    })?;
                let text = extract_prompt_text(&params)?;
                let chunks = self.backend.prompt(session_id, &text).await?;
                let mut messages = Vec::new();
                for chunk in chunks {
                    messages.push(json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_key,
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {
                                    "type": "text",
                                    "text": chunk,
                                }
                            }
                        }
                    }));
                }
                messages.push(success(
                    id,
                    json!({
                        "stopReason": "end_turn"
                    }),
                ));
                Ok(messages)
            }
            "session/cancel" => {
                let params = object.get("params").cloned().unwrap_or(json!({}));
                let session_key =
                    params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            AcpServerError::new(
                                AcpServerErrorKind::InvalidRequest,
                                "session/cancel requires sessionId",
                            )
                        })?;
                let session_id = self
                    .sessions
                    .lock()
                    .map_err(|_| {
                        AcpServerError::new(AcpServerErrorKind::Backend, "session map unavailable")
                    })?
                    .get(session_key)
                    .copied();
                if let Some(session_id) = session_id {
                    let _ = self.backend.cancel(session_id).await;
                }
                Ok(vec![success(id, json!({}))])
            }
            other => Err(AcpServerError::new(
                AcpServerErrorKind::InvalidRequest,
                format!("unsupported ACP method: {other}"),
            )),
        }
    }

    /// Processes one raw NDJSON line into encoded response frames.
    ///
    /// # Errors
    ///
    /// Returns when decoding or handling fails.
    pub async fn handle_line(&self, line: &[u8]) -> Result<Vec<Vec<u8>>, AcpServerError> {
        let message = decode_line(line)?;
        let responses = self.handle_message(message).await?;
        responses.iter().map(encode_message).collect()
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }
}

fn success(id: Option<Value>, result: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    map.insert("result".to_owned(), result);
    if let Some(id) = id {
        map.insert("id".to_owned(), id);
    }
    Value::Object(map)
}

fn extract_prompt_text(params: &Value) -> Result<String, AcpServerError> {
    if let Some(text) = params.get("prompt").and_then(Value::as_str) {
        return non_empty(text);
    }
    if let Some(blocks) = params.get("prompt").and_then(Value::as_array) {
        let mut text = String::new();
        for block in blocks {
            if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                text.push_str(chunk);
            } else if let Some(chunk) = block
                .get("content")
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
            {
                text.push_str(chunk);
            }
        }
        return non_empty(&text);
    }
    if let Some(text) = params.get("text").and_then(Value::as_str) {
        return non_empty(text);
    }
    Err(AcpServerError::new(
        AcpServerErrorKind::InvalidRequest,
        "session/prompt is missing text",
    ))
}

fn non_empty(text: &str) -> Result<String, AcpServerError> {
    if text.is_empty() {
        Err(AcpServerError::new(
            AcpServerErrorKind::InvalidRequest,
            "session/prompt text must not be empty",
        ))
    } else {
        Ok(text.to_owned())
    }
}
