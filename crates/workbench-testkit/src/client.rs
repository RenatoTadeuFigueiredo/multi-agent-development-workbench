use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tempfile::TempDir;
use tokio::{net::UnixListener, task::JoinHandle};
use tokio_util::codec::Framed;
use uuid::Uuid;
use workbench_daemon::{Application, ipc::serve_connection};
use workbench_protocol::{
    ClientCommand, Command, NdjsonCodec, PROTOCOL_V1, ServerReply, SessionEvent,
    command::InitializeParams,
};

/// Deterministic protocol command factory for future daemon contract suites.
#[derive(Clone, Debug)]
pub struct ClientCommandFactory {
    session_id: Uuid,
}

impl ClientCommandFactory {
    #[must_use]
    pub fn new(session_id: Uuid) -> Self {
        Self { session_id }
    }

    #[must_use]
    pub fn with_new_session() -> Self {
        Self::new(Uuid::now_v7())
    }

    #[must_use]
    pub fn command(&self, command: Command) -> ClientCommand {
        ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id: Uuid::now_v7(),
            session_id: command.requires_session().then_some(self.session_id),
            command,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> Uuid {
        self.session_id
    }
}

pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    Ok(encoded)
}

pub fn decode_reply<T: DeserializeOwned>(
    frame: &[u8],
) -> Result<ServerReply<T>, serde_json::Error> {
    let payload = frame.strip_suffix(b"\n").unwrap_or(frame);
    serde_json::from_slice(payload)
}

pub fn round_trip_command(command: &ClientCommand) -> Result<ClientCommand, serde_json::Error> {
    serde_json::from_slice(&serde_json::to_vec(command)?)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InboundMessage {
    Reply(ServerReply<Value>),
    Event(SessionEvent),
}

type TestTransport = Framed<tokio::net::UnixStream, NdjsonCodec<InboundMessage, ClientCommand>>;

/// A deterministic protocol client used by daemon contract and SLO suites.
pub struct ProtocolTestClient {
    transport: TestTransport,
    pending_events: VecDeque<SessionEvent>,
}

impl ProtocolTestClient {
    /// Connects to a local test daemon and negotiates protocol version 1.
    pub async fn connect(endpoint: &Path, client_name: &str) -> Result<Self, TestClientError> {
        let stream = tokio::net::UnixStream::connect(endpoint).await?;
        let mut client = Self {
            transport: Framed::new(stream, NdjsonCodec::default()),
            pending_events: VecDeque::new(),
        };
        let result = client
            .call(ClientCommand {
                protocol: PROTOCOL_V1.to_owned(),
                request_id: Uuid::now_v7(),
                session_id: None,
                command: Command::Initialize(InitializeParams {
                    client_name: client_name.to_owned(),
                    client_version: env!("CARGO_PKG_VERSION").to_owned(),
                    supported_protocols: vec![PROTOCOL_V1.to_owned()],
                }),
            })
            .await?;
        if result["selected_protocol"] != PROTOCOL_V1 {
            return Err(TestClientError::InvalidResult);
        }
        Ok(client)
    }

    /// Sends one command and returns its correlated JSON result.
    pub async fn call(&mut self, command: ClientCommand) -> Result<Value, TestClientError> {
        let request_id = command.request_id;
        self.transport.send(command).await?;
        loop {
            match self
                .transport
                .next()
                .await
                .ok_or(TestClientError::Disconnected)??
            {
                InboundMessage::Event(event) => self.pending_events.push_back(event),
                InboundMessage::Reply(ServerReply::Success {
                    request_id: actual,
                    result,
                }) => {
                    if actual != request_id {
                        return Err(TestClientError::CorrelationMismatch);
                    }
                    return Ok(result);
                }
                InboundMessage::Reply(ServerReply::Failure {
                    request_id: actual,
                    error,
                }) => {
                    if actual != request_id {
                        return Err(TestClientError::CorrelationMismatch);
                    }
                    return Err(TestClientError::Protocol(error));
                }
            }
        }
    }

    /// Returns the next queued or live session event.
    pub async fn next_event(&mut self) -> Result<SessionEvent, TestClientError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }
        match self
            .transport
            .next()
            .await
            .ok_or(TestClientError::Disconnected)??
        {
            InboundMessage::Event(event) => Ok(event),
            InboundMessage::Reply(ServerReply::Failure { error, .. }) => {
                Err(TestClientError::Protocol(error))
            }
            InboundMessage::Reply(ServerReply::Success { .. }) => {
                Err(TestClientError::UnexpectedReply)
            }
        }
    }
}

/// Owner of an offline Unix-socket daemon fixture.
pub struct LocalDaemonHarness {
    endpoint: PathBuf,
    _directory: TempDir,
    server: JoinHandle<()>,
}

impl LocalDaemonHarness {
    /// Starts a same-user local daemon over an owner-only temporary endpoint.
    pub fn start(application: Arc<Application>) -> io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("workbench-contract-")
            .tempdir_in("/tmp")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let endpoint = directory.path().join("workbench.sock");
        let listener = UnixListener::bind(&endpoint)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
        }
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let application = Arc::clone(&application);
                drop(tokio::spawn(async move {
                    let _ignored = serve_connection(application, stream).await;
                }));
            }
        });
        Ok(Self {
            endpoint,
            _directory: directory,
            server,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }
}

impl Drop for LocalDaemonHarness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TestClientError {
    #[error("test client I/O failed")]
    Io(#[from] io::Error),
    #[error("test client framing failed")]
    Codec(#[from] workbench_protocol::ProtocolCodecError),
    #[error("daemon rejected the test command")]
    Protocol(workbench_protocol::ProtocolError),
    #[error("daemon reply did not correlate to the command")]
    CorrelationMismatch,
    #[error("daemon returned an invalid initialization result")]
    InvalidResult,
    #[error("daemon sent an unexpected success reply")]
    UnexpectedReply,
    #[error("daemon disconnected")]
    Disconnected,
}
