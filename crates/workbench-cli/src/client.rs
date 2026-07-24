use std::path::Path;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;
use tokio::net::UnixStream;
use tokio_util::codec::Framed;
use uuid::Uuid;
use workbench_protocol::{
    ClientCommand, Command, ErrorCode, NdjsonCodec, PROTOCOL_V1, ServerReply, SessionEvent,
    command::InitializeParams,
    response::{
        ApprovalResult, AttachSessionResult, ControlResult, CreateSessionResult, DeleteResult,
        ExportResult, InitializeResult, ListSessionsResult, PromptResult, ReconciliationResult,
        SessionResult, StatusResult,
    },
};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InboundMessage {
    Reply(ServerReply<Value>),
    Event(SessionEvent),
}

type ClientTransport = Framed<UnixStream, NdjsonCodec<InboundMessage, ClientCommand>>;

pub struct ProtocolClient {
    transport: ClientTransport,
    pending_events: std::collections::VecDeque<SessionEvent>,
    seen_event_ids: std::collections::HashSet<Uuid>,
}

impl ProtocolClient {
    /// Connects to a local daemon and negotiates protocol version 1.
    ///
    /// # Errors
    ///
    /// Returns an error when the endpoint cannot be opened, the initialization
    /// request fails, or the daemon selects an incompatible protocol.
    pub async fn connect(endpoint: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(endpoint).await?;
        let mut client = Self {
            transport: Framed::new(stream, NdjsonCodec::default()),
            pending_events: std::collections::VecDeque::new(),
            seen_event_ids: std::collections::HashSet::new(),
        };
        let request_id = Uuid::now_v7();
        let _result: InitializeResult = client
            .call(ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id,
                session_id: None,
                command: Command::Initialize(InitializeParams {
                    client_name: "workbench-cli".to_owned(),
                    client_version: env!("CARGO_PKG_VERSION").to_owned(),
                    supported_protocols: vec![PROTOCOL_V1.to_owned()],
                }),
            })
            .await?;
        Ok(client)
    }

    /// Sends one command and waits for its correlated reply.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, correlation mismatch, daemon
    /// rejection, or disconnect before a reply.
    pub async fn call<T>(&mut self, command: ClientCommand) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
    {
        let expected_request_id = command.request_id;
        self.transport.send(command).await?;
        loop {
            let message = self
                .transport
                .next()
                .await
                .ok_or(ClientError::Disconnected)??;
            match message {
                InboundMessage::Event(event) => {
                    validate_event(&event)?;
                    self.pending_events.push_back(event);
                }
                InboundMessage::Reply(ServerReply::Success { request_id, result }) => {
                    require_correlation(expected_request_id, request_id)?;
                    return serde_json::from_value(result)
                        .map_err(|error| ClientError::InvalidResult(error.to_string()));
                }
                InboundMessage::Reply(ServerReply::Failure { request_id, error }) => {
                    if !is_pre_correlation_error(error.code) {
                        require_correlation(expected_request_id, request_id)?;
                    }
                    return Err(ClientError::Protocol(error));
                }
            }
        }
    }

    /// Sends one command and validates its result against the method DTO.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::call`] and rejects a result that does
    /// not match the `AsyncAPI` schema selected by the command method.
    pub async fn call_validated(&mut self, command: ClientCommand) -> Result<Value, ClientError> {
        let method = command.command.clone();
        let value: Value = self.call(command).await?;
        validate_method_result(&method, &value)?;
        Ok(value)
    }

    /// Receives the next session event after an attach command.
    ///
    /// # Errors
    ///
    /// Returns an error on framing failure, disconnect, or an unexpected reply
    /// when no request is outstanding.
    pub async fn next_event(&mut self) -> Result<SessionEvent, ClientError> {
        while let Some(event) = self.pending_events.pop_front() {
            if self.seen_event_ids.insert(event.event_id) {
                return Ok(event);
            }
        }
        loop {
            let message = self
                .transport
                .next()
                .await
                .ok_or(ClientError::Disconnected)??;
            match message {
                InboundMessage::Event(event) => {
                    validate_event(&event)?;
                    if self.seen_event_ids.insert(event.event_id) {
                        return Ok(event);
                    }
                }
                InboundMessage::Reply(ServerReply::Failure { error, .. }) => {
                    return Err(ClientError::Protocol(error));
                }
                InboundMessage::Reply(ServerReply::Success { .. }) => {
                    return Err(ClientError::UnexpectedReply);
                }
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("local daemon I/O failed")]
    Io(#[from] std::io::Error),
    #[error("local protocol framing failed")]
    Codec(#[from] workbench_protocol::ProtocolCodecError),
    #[error("daemon rejected the command: {0:?}")]
    Protocol(workbench_protocol::ProtocolError),
    #[error("daemon selected an incompatible protocol")]
    IncompatibleProtocol,
    #[error("daemon returned a result that violates the method contract")]
    InvalidResult(String),
    #[error("daemon returned an event that violates the protocol contract")]
    InvalidEvent,
    #[error("daemon reply did not match the request")]
    CorrelationMismatch,
    #[error("daemon sent a reply while the client expected session events")]
    UnexpectedReply,
    #[error("daemon disconnected before replying")]
    Disconnected,
}

fn validate_event(event: &SessionEvent) -> Result<(), ClientError> {
    event.validate().map_err(|_| ClientError::InvalidEvent)
}

fn validate_method_result(command: &Command, value: &Value) -> Result<(), ClientError> {
    macro_rules! require {
        ($type:ty) => {
            serde_json::from_value::<$type>(value.clone())
                .map(|_| ())
                .map_err(|error| ClientError::InvalidResult(error.to_string()))
        };
    }
    match command {
        Command::Initialize(_) => require!(InitializeResult),
        Command::StatusGet(_) => require!(StatusResult),
        Command::SessionCreate(_) => require!(CreateSessionResult),
        Command::SessionList(_) => require!(ListSessionsResult),
        Command::SessionGet(_) => require!(SessionResult),
        Command::SessionAttach(_) => require!(AttachSessionResult),
        Command::SessionPrompt(_) => require!(PromptResult),
        Command::SessionPause(_)
        | Command::SessionResume(_)
        | Command::SessionRedirect(_)
        | Command::SessionCancel(_) => require!(ControlResult),
        Command::SessionApprovalResolve(_) => require!(ApprovalResult),
        Command::SessionReconcile(_) => require!(ReconciliationResult),
        Command::SessionExport(_) => require!(ExportResult),
        Command::SessionDelete(_) => require!(DeleteResult),
    }
}

fn require_correlation(expected: Uuid, actual: Uuid) -> Result<(), ClientError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ClientError::CorrelationMismatch)
    }
}

const fn is_pre_correlation_error(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::InvalidRequest | ErrorCode::UnsupportedVersion | ErrorCode::FrameTooLarge
    )
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::net::UnixListener;
    use tokio_util::codec::Framed;
    use workbench_protocol::{
        ClientCommand, Command, NdjsonCodec, PROTOCOL_V1, ServerReply, command::EmptyParams,
    };

    use super::*;

    #[tokio::test]
    async fn negotiates_and_correlates_one_shot_commands() {
        let root = TempDir::new().expect("temporary endpoint");
        let endpoint = root.path().join("workbench.sock");
        let listener = UnixListener::bind(&endpoint).expect("bind endpoint");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("client");
            let mut transport: Framed<_, NdjsonCodec<ClientCommand, ServerReply<Value>>> =
                Framed::new(stream, NdjsonCodec::default());
            let initialize = transport
                .next()
                .await
                .expect("initialize frame")
                .expect("initialize command");
            transport
                .send(ServerReply::Success {
                    request_id: initialize.request_id,
                    result: json!({
                        "selected_protocol": "workbench/1",
                        "max_frame_bytes": 8_388_608,
                        "max_client_queue_events": 1_024,
                        "max_client_queue_bytes": 8_388_608
                    }),
                })
                .await
                .expect("initialize result");
            let status = transport
                .next()
                .await
                .expect("status frame")
                .expect("status command");
            transport
                .send(ServerReply::Success {
                    request_id: status.request_id,
                    result: json!({
                        "daemon_version": "0.1.0",
                        "protocol": "workbench/1",
                        "storage_schema_version": 1,
                        "key_store": "available",
                        "migration": "ready",
                        "active_sessions": 0,
                        "adapters": []
                    }),
                })
                .await
                .expect("status result");
        });

        let mut client = ProtocolClient::connect(&endpoint)
            .await
            .expect("protocol client");
        let result: workbench_protocol::response::StatusResult = client
            .call(ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::StatusGet(EmptyParams::default()),
            })
            .await
            .expect("status result");

        assert_eq!(result.active_sessions, 0);
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn accepts_transport_failures_that_precede_request_correlation() {
        let root = TempDir::new().expect("temporary endpoint");
        let endpoint = root.path().join("workbench.sock");
        let listener = UnixListener::bind(&endpoint).expect("bind endpoint");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("client");
            let mut transport: Framed<_, NdjsonCodec<ClientCommand, ServerReply<Value>>> =
                Framed::new(stream, NdjsonCodec::default());
            let initialize = transport
                .next()
                .await
                .expect("initialize frame")
                .expect("initialize command");
            transport
                .send(ServerReply::Success {
                    request_id: initialize.request_id,
                    result: json!({
                        "selected_protocol": "workbench/1",
                        "max_frame_bytes": 8_388_608,
                        "max_client_queue_events": 1_024,
                        "max_client_queue_bytes": 8_388_608
                    }),
                })
                .await
                .expect("initialize result");
            let _status = transport
                .next()
                .await
                .expect("status frame")
                .expect("status command");
            transport
                .send(ServerReply::Failure {
                    request_id: Uuid::now_v7(),
                    error: workbench_protocol::ProtocolError {
                        code: ErrorCode::FrameTooLarge,
                        message: "protocol frame was rejected".to_owned(),
                        retryable: false,
                        correlation_id: Uuid::now_v7(),
                    },
                })
                .await
                .expect("transport failure");
        });

        let mut client = ProtocolClient::connect(&endpoint)
            .await
            .expect("protocol client");
        let error = client
            .call::<Value>(ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::StatusGet(EmptyParams::default()),
            })
            .await
            .expect_err("transport failure");
        let ClientError::Protocol(error) = error else {
            panic!("pre-correlation protocol error expected");
        };
        assert_eq!(error.code, ErrorCode::FrameTooLarge);
        server.await.expect("server task");
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn attach_streams_events_and_deduplicates_stable_event_ids() {
        let root = TempDir::new().expect("temporary endpoint");
        let endpoint = root.path().join("workbench.sock");
        let listener = UnixListener::bind(&endpoint).expect("bind endpoint");
        let session_id = Uuid::now_v7();
        let first_event = event(session_id, 1);
        let second_event = event(session_id, 2);
        let first_event_id = first_event.event_id;
        let second_event_id = second_event.event_id;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("client");
            let mut transport: Framed<_, NdjsonCodec<ClientCommand, Value>> =
                Framed::new(stream, NdjsonCodec::default());
            let initialize = transport
                .next()
                .await
                .expect("initialize frame")
                .expect("initialize command");
            transport
                .send(json!({
                    "protocol": PROTOCOL_V1,
                    "request_id": initialize.request_id,
                    "ok": true,
                    "result": {
                        "selected_protocol": PROTOCOL_V1,
                        "max_frame_bytes": 8_388_608,
                        "max_client_queue_events": 1_024,
                        "max_client_queue_bytes": 8_388_608
                    }
                }))
                .await
                .expect("initialize result");
            let attach = transport
                .next()
                .await
                .expect("attach frame")
                .expect("attach command");
            transport
                .send(json!({
                    "protocol": PROTOCOL_V1,
                    "request_id": attach.request_id,
                    "ok": true,
                    "result": {
                        "session_id": session_id,
                        "state": "ready",
                        "replay_after_sequence": 0,
                        "last_sequence": 2
                    }
                }))
                .await
                .expect("attach result");
            for event in [&first_event, &first_event, &second_event] {
                transport
                    .send(serde_json::to_value(event).expect("event JSON"))
                    .await
                    .expect("stream event");
            }
            transport
                .send(json!({
                    "protocol": PROTOCOL_V1,
                    "request_id": Uuid::now_v7(),
                    "ok": false,
                    "error": {
                        "code": "client_lagged",
                        "message": "client exceeded the bounded event queue",
                        "retryable": false,
                        "correlation_id": Uuid::now_v7()
                    }
                }))
                .await
                .expect("stream error");
        });

        let mut client = ProtocolClient::connect(&endpoint)
            .await
            .expect("protocol client");
        let result = client
            .call_validated(ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: Some(session_id),
                command: Command::SessionAttach(workbench_protocol::command::AttachSessionParams {
                    after_sequence: 0,
                }),
            })
            .await
            .expect("attach result");
        assert_eq!(result["last_sequence"], 2);
        assert_eq!(
            client.next_event().await.expect("first event").event_id,
            first_event_id
        );
        assert_eq!(
            client.next_event().await.expect("second event").event_id,
            second_event_id
        );
        let ClientError::Protocol(error) = client
            .next_event()
            .await
            .expect_err("lagged stream must fail")
        else {
            panic!("protocol stream error expected");
        };
        assert_eq!(error.code, workbench_protocol::ErrorCode::ClientLagged);
        server.await.expect("server task");
    }

    fn event(session_id: Uuid, sequence: u64) -> SessionEvent {
        SessionEvent {
            protocol: PROTOCOL_V1.to_owned(),
            event_id: Uuid::now_v7(),
            session_id,
            sequence,
            causation_request_id: None,
            kind: workbench_protocol::EventKind::ProviderEvent,
            occurred_at: "1970-01-01T00:00:00Z".to_owned(),
            data: json!({"sequence": sequence}),
        }
    }
}
