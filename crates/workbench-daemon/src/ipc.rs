use std::{
    fs::{self, Permissions},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
};

use futures_util::{SinkExt, StreamExt};
use rustix::process::getuid;
use serde::Serialize;
use serde_json::Value;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::Framed;
use uuid::Uuid;
use workbench_protocol::{
    ClientCommand, Command, ErrorCode, NdjsonCodec, ProtocolCodecError, ProtocolError, ServerReply,
    SessionEvent,
};

use crate::{
    Application, ClientContext,
    runtime_paths::{RuntimePaths, SingleDaemonLock},
    subscription::{SessionSubscription, SubscriptionItem},
};

#[derive(Serialize)]
#[serde(untagged)]
enum WireMessage {
    Reply(ServerReply<Value>),
    Event(SessionEvent),
}

pub struct BoundEndpoint {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl BoundEndpoint {
    /// Binds the owner-only local endpoint after conservative stale proof.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the endpoint is occupied, unsafe, or cannot
    /// be created with the required permissions.
    pub fn bind(paths: &RuntimePaths, _daemon_lock: &SingleDaemonLock) -> io::Result<Self> {
        remove_proven_stale_endpoint(paths)?;
        let listener = UnixListener::bind(&paths.endpoint)?;
        fs::set_permissions(&paths.endpoint, Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&paths.endpoint)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != getuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            let _ignored = fs::remove_file(&paths.endpoint);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "IPC endpoint ownership or permissions are unsafe",
            ));
        }
        Ok(Self {
            listener,
            path: paths.endpoint.clone(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Accepts one local Unix client.
    ///
    /// # Errors
    ///
    /// Returns the listener I/O error when the accept operation fails.
    pub async fn accept(&self) -> io::Result<(UnixStream, tokio::net::unix::SocketAddr)> {
        self.listener.accept().await
    }
}

impl Drop for BoundEndpoint {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ignored = fs::remove_file(&self.path);
        }
    }
}

fn remove_proven_stale_endpoint(paths: &RuntimePaths) -> io::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(&paths.endpoint) else {
        return Ok(());
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "occupied IPC endpoint cannot be proven stale",
        ));
    }
    match std::os::unix::net::UnixStream::connect(&paths.endpoint) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "an IPC listener is accepting on the endpoint",
        )),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                fs::remove_file(&paths.endpoint)
            }
        }
        Err(error) => Err(io::Error::new(
            error.kind(),
            "occupied IPC endpoint cannot be proven stale",
        )),
    }
}

/// Serves one verified same-user protocol connection.
///
/// # Errors
///
/// Returns an I/O error for peer verification, framing, or socket failures.
pub async fn serve_connection(application: Arc<Application>, stream: UnixStream) -> io::Result<()> {
    serve_connection_for_uid(application, stream, getuid().as_raw()).await
}

async fn serve_connection_for_uid(
    application: Arc<Application>,
    stream: UnixStream,
    expected_uid: u32,
) -> io::Result<()> {
    let credentials = stream.peer_cred()?;
    if credentials.uid() != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC peer is not owned by the daemon user",
        ));
    }
    let uid = credentials.uid();
    let mut framed = Framed::new(stream, NdjsonCodec::<ClientCommand, WireMessage>::default());
    let mut initialized = false;
    let mut context = ClientContext {
        uid,
        client_name: "uninitialized".to_owned(),
    };
    let mut subscription: Option<SessionSubscription> = None;

    loop {
        if let Some(active_subscription) = subscription.as_mut() {
            tokio::select! {
                inbound = framed.next() => {
                    if !handle_inbound(
                        &application,
                        &mut framed,
                        inbound,
                        &mut initialized,
                        &mut context,
                        &mut subscription,
                    ).await? {
                        return Ok(());
                    }
                }
                item = active_subscription.next() => {
                    match item {
                        Some(SubscriptionItem::Event(event)) => {
                            framed.send(WireMessage::Event(event)).await.map_err(codec_io)?;
                        }
                        Some(SubscriptionItem::Lagged) => {
                            let reply = lagged_reply();
                            let _ignored = framed.send(WireMessage::Reply(reply)).await;
                            return Ok(());
                        }
                        None => subscription = None,
                    }
                }
            }
        } else {
            let inbound = framed.next().await;
            if !handle_inbound(
                &application,
                &mut framed,
                inbound,
                &mut initialized,
                &mut context,
                &mut subscription,
            )
            .await?
            {
                return Ok(());
            }
        }
    }
}

async fn handle_inbound(
    application: &Arc<Application>,
    framed: &mut Framed<UnixStream, NdjsonCodec<ClientCommand, WireMessage>>,
    inbound: Option<Result<ClientCommand, ProtocolCodecError>>,
    initialized: &mut bool,
    context: &mut ClientContext,
    subscription: &mut Option<SessionSubscription>,
) -> io::Result<bool> {
    let command = match inbound {
        Some(Ok(command)) => command,
        Some(Err(error)) => {
            let code = match error {
                ProtocolCodecError::FrameTooLarge => ErrorCode::FrameTooLarge,
                ProtocolCodecError::UnsupportedVersion => ErrorCode::UnsupportedVersion,
                _ => ErrorCode::InvalidRequest,
            };
            let reply = failure_reply(code, "protocol frame was rejected");
            let _ignored = framed.send(WireMessage::Reply(reply)).await;
            return Ok(false);
        }
        None => return Ok(false),
    };
    let is_initialize = matches!(&command.command, Command::Initialize(_));
    if (!*initialized && !is_initialize) || (*initialized && is_initialize) {
        let reply = ServerReply::Failure {
            request_id: command.request_id,
            error: ProtocolError {
                code: ErrorCode::InvalidRequest,
                message: if *initialized {
                    "connection is already initialized"
                } else {
                    "initialize must be the first command"
                }
                .to_owned(),
                retryable: false,
                correlation_id: Uuid::now_v7(),
            },
        };
        framed
            .send(WireMessage::Reply(reply))
            .await
            .map_err(codec_io)?;
        return Ok(true);
    }
    let client_name = match &command.command {
        Command::Initialize(params) => Some(params.client_name.clone()),
        _ => None,
    };
    let result = application.dispatch(command, context).await;
    let success = matches!(result.reply, ServerReply::Success { .. });
    framed
        .send(WireMessage::Reply(result.reply))
        .await
        .map_err(codec_io)?;
    if is_initialize && success {
        *initialized = true;
        if let Some(client_name) = client_name {
            context.client_name = client_name;
        }
    }
    if result.subscription.is_some() {
        *subscription = result.subscription;
    }
    Ok(true)
}

fn lagged_reply() -> ServerReply<Value> {
    failure_reply(
        ErrorCode::ClientLagged,
        "client exceeded the bounded event queue",
    )
}

fn failure_reply(code: ErrorCode, message: &str) -> ServerReply<Value> {
    ServerReply::Failure {
        request_id: Uuid::now_v7(),
        error: ProtocolError {
            code,
            message: message.to_owned(),
            retryable: false,
            correlation_id: Uuid::now_v7(),
        },
    }
}

fn codec_io(error: ProtocolCodecError) -> io::Error {
    match error {
        ProtocolCodecError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::runtime_paths::RuntimePaths;

    #[tokio::test]
    async fn incompatible_protocol_major_returns_the_stable_transport_error() {
        let application = Application::in_memory(
            crate::StartupConfiguration::safe_builtins().expect("startup"),
            crate::FakeBehavior::default(),
        )
        .expect("application");
        let (mut client, server) = UnixStream::pair().expect("socket pair");
        let request_id = Uuid::now_v7();
        let daemon = tokio::spawn(serve_connection(application, server));
        client
            .write_all(
                format!(
                    "{{\"protocol\":\"workbench/2\",\"request_id\":\"{request_id}\",\"method\":\"status.get\",\"params\":{{}}}}\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write incompatible command");
        client.shutdown().await.expect("finish request");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("read response");
        let value: Value = serde_json::from_slice(&response).expect("failure reply");

        assert_eq!(value["error"]["code"], "unsupported_version");
        daemon
            .await
            .expect("daemon task")
            .expect("serve connection");
    }

    #[tokio::test]
    async fn unauthorized_peer_is_rejected_before_a_command_is_read() {
        let application = Application::in_memory(
            crate::StartupConfiguration::safe_builtins().expect("startup"),
            crate::FakeBehavior::default(),
        )
        .expect("application");
        let (_client, server) = UnixStream::pair().expect("socket pair");
        let wrong_uid = getuid().as_raw().wrapping_add(1);

        let error = serve_connection_for_uid(application, server, wrong_uid)
            .await
            .expect_err("unauthorized peer");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn endpoint_and_peer_are_owner_only() {
        let root = TempDir::new().expect("root");
        let root = root.path().canonicalize().expect("canonical root");
        let paths = RuntimePaths::from_parts(
            root.join("config.yaml"),
            root.join("state"),
            root.join("runtime"),
        )
        .expect("paths");
        paths.prepare().expect("directories");
        let daemon_lock = SingleDaemonLock::acquire(&paths).expect("lock");
        let endpoint = BoundEndpoint::bind(&paths, &daemon_lock).expect("endpoint");
        assert_eq!(
            fs::metadata(&paths.endpoint)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let client = UnixStream::connect(&paths.endpoint).await.expect("client");
        let (server, _) = endpoint.accept().await.expect("server");
        assert_eq!(
            client.peer_cred().expect("client peer").uid(),
            getuid().as_raw()
        );
        assert_eq!(
            server.peer_cred().expect("server peer").uid(),
            getuid().as_raw()
        );
    }

    #[test]
    fn refuses_to_replace_an_owned_active_listener() {
        let root = TempDir::new().expect("root");
        let root = root.path().canonicalize().expect("canonical root");
        let paths = RuntimePaths::from_parts(
            root.join("config.yaml"),
            root.join("state"),
            root.join("runtime"),
        )
        .expect("paths");
        paths.prepare().expect("directories");
        let daemon_lock = SingleDaemonLock::acquire(&paths).expect("lock");
        let listener = std::os::unix::net::UnixListener::bind(&paths.endpoint)
            .expect("foreign active listener");
        fs::set_permissions(&paths.endpoint, Permissions::from_mode(0o600))
            .expect("private endpoint");

        let Err(error) = BoundEndpoint::bind(&paths, &daemon_lock) else {
            panic!("active listener must not be replaced");
        };

        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(paths.endpoint.exists());
        drop(listener);
    }

    #[tokio::test]
    async fn replaces_an_owned_private_stale_socket_after_connection_proof() {
        let root = TempDir::new().expect("root");
        let root = root.path().canonicalize().expect("canonical root");
        let paths = RuntimePaths::from_parts(
            root.join("config.yaml"),
            root.join("state"),
            root.join("runtime"),
        )
        .expect("paths");
        paths.prepare().expect("directories");
        let daemon_lock = SingleDaemonLock::acquire(&paths).expect("lock");
        let listener =
            std::os::unix::net::UnixListener::bind(&paths.endpoint).expect("stale listener");
        fs::set_permissions(&paths.endpoint, Permissions::from_mode(0o600))
            .expect("private endpoint");
        drop(listener);

        let replacement =
            BoundEndpoint::bind(&paths, &daemon_lock).expect("replace stale endpoint");

        assert!(paths.endpoint.exists());
        drop(replacement);
    }
}
