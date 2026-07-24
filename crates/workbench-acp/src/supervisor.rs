use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

use crate::{
    ACP_PROTOCOL_VERSION, AcpCapabilities, AcpError, AcpErrorKind, AcpSession, AdapterHealth,
    AuthenticationStatus, PromptExecution,
    protocol::{parse_initialize, parse_session},
    transport::{Connection, channel, spawn_reader, spawn_writer},
};

const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const FATAL_QUEUE_DEPTH: usize = 4;

#[derive(Debug, Clone)]
pub struct GrokLaunchProfile {
    executable: PathBuf,
    workspace: PathBuf,
    shutdown_grace: Duration,
    request_timeout: Duration,
}

impl GrokLaunchProfile {
    pub fn new(executable: impl Into<PathBuf>, workspace: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            workspace: workspace.into(),
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    #[must_use]
    pub const fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub reaped: bool,
    pub forced: bool,
}

struct ShutdownCommand {
    respond: oneshot::Sender<ShutdownReport>,
}

pub struct GrokAcpClient {
    connection: Arc<Connection>,
    capabilities: AcpCapabilities,
    workspace: PathBuf,
    request_timeout: Duration,
    health: watch::Receiver<AdapterHealth>,
    shutdown: mpsc::Sender<ShutdownCommand>,
    shutdown_grace: Duration,
    final_report: watch::Receiver<Option<ShutdownReport>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl GrokAcpClient {
    /// Starts the configured child and completes ACP version negotiation.
    ///
    /// # Errors
    ///
    /// Returns a redacted configuration, spawn, transport, authentication, or
    /// compatibility error. Failed initialization always triggers child
    /// shutdown before this function returns.
    pub async fn connect(profile: GrokLaunchProfile) -> Result<Self, AcpError> {
        if profile.request_timeout.is_zero() || profile.shutdown_grace.is_zero() {
            return Err(invalid_configuration());
        }
        let executable = canonical_executable(&profile.executable)?;
        let workspace = canonical_workspace(&profile.workspace)?;
        let mut command = Command::new(executable);
        command
            .args(["agent", "--no-leader", "stdio"])
            .env("GROK_DISABLE_AUTOUPDATER", "1")
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| spawn_failed())?;
        let stdin = child.stdin.take().ok_or_else(spawn_failed)?;
        let stdout = child.stdout.take().ok_or_else(spawn_failed)?;
        let stderr = child.stderr.take().ok_or_else(spawn_failed)?;
        let (health_tx, health) = watch::channel(AdapterHealth::Starting);
        let (fatal_tx, fatal_rx) = mpsc::channel(FATAL_QUEUE_DEPTH);
        let (writer_tx, writer_rx) = channel();
        let connection = Connection::new(writer_tx, health_tx.clone(), fatal_tx);
        let reader = spawn_reader(stdout, Arc::clone(&connection));
        let writer = spawn_writer(stdin, writer_rx, Arc::clone(&connection));
        let stderr = spawn_stderr_drain(stderr);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let (report_tx, final_report) = watch::channel(None);
        let supervisor = spawn_supervisor(
            child,
            shutdown_rx,
            fatal_rx,
            profile.shutdown_grace,
            Arc::clone(&connection),
            report_tx,
        );
        let mut client = Self {
            connection,
            capabilities: AcpCapabilities {
                load_session: false,
                authentication: AuthenticationStatus::Unavailable,
                agent_name: None,
                agent_version: None,
            },
            workspace,
            request_timeout: profile.request_timeout,
            health,
            shutdown: shutdown_tx,
            shutdown_grace: profile.shutdown_grace,
            final_report,
            tasks: Mutex::new(vec![reader, writer, stderr, supervisor]),
        };
        match client.initialize().await {
            Ok(capabilities) => {
                let health = match capabilities.authentication {
                    AuthenticationStatus::Available => AdapterHealth::Available,
                    AuthenticationStatus::InteractiveRequired => {
                        AdapterHealth::AuthenticationRequired
                    }
                    AuthenticationStatus::Unavailable => AdapterHealth::Unavailable,
                };
                let _ignored = health_tx.send(health);
                client.capabilities = capabilities;
                Ok(client)
            }
            Err(error) => {
                let _ignored = health_tx.send(
                    if matches!(
                        error.kind(),
                        AcpErrorKind::IncompatibleProtocol | AcpErrorKind::CapabilityUnavailable
                    ) {
                        AdapterHealth::Incompatible
                    } else {
                        AdapterHealth::Unavailable
                    },
                );
                if client.shutdown().await.reaped {
                    Err(error)
                } else {
                    Err(AcpError::new(
                        AcpErrorKind::ReapFailed,
                        "ACP provider process could not be reaped",
                    ))
                }
            }
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> &AcpCapabilities {
        &self.capabilities
    }

    #[must_use]
    pub fn health(&self) -> AdapterHealth {
        *self.health.borrow()
    }

    /// Creates a provider session rooted in this client's canonical workspace.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when authentication or the transport is
    /// unavailable, or when the agent returns an invalid session response.
    pub async fn new_session(&self, model_id: Option<&str>) -> Result<AcpSession, AcpError> {
        self.require_available()?;
        let mut metadata = serde_json::Map::new();
        if let Some(model_id) = model_id {
            metadata.insert("modelId".to_owned(), json!(model_id));
        }
        let result = self
            .connection
            .request(
                "session/new",
                json!({
                    "cwd": self.workspace,
                    "mcpServers": [],
                    "_meta": metadata
                }),
                self.request_timeout,
            )
            .await?;
        parse_session(&result)
    }

    /// Loads an existing provider-owned session into this child process.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when session loading was not advertised,
    /// authentication is required, or the transport rejects the request.
    pub async fn load_session(&self, session_id: &str) -> Result<AcpSession, AcpError> {
        self.require_available()?;
        if !self.capabilities.load_session {
            return Err(AcpError::new(
                AcpErrorKind::CapabilityUnavailable,
                "ACP session loading is unavailable",
            ));
        }
        self.connection
            .request(
                "session/load",
                json!({
                    "sessionId": session_id,
                    "cwd": self.workspace,
                    "mcpServers": []
                }),
                self.request_timeout,
            )
            .await?;
        crate::protocol::session_from_id(session_id)
    }

    /// Starts one streaming prompt for a provider session.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for empty input, unavailable authentication or
    /// transport, or a second concurrent prompt for the same provider session.
    pub async fn prompt(
        &self,
        session: &AcpSession,
        text: &str,
    ) -> Result<PromptExecution, AcpError> {
        self.require_available()?;
        if text.is_empty() {
            return Err(AcpError::new(
                AcpErrorKind::ProtocolViolation,
                "ACP prompt must not be empty",
            ));
        }
        self.connection
            .start_prompt(
                session.id(),
                json!({
                    "sessionId": session.id(),
                    "prompt": [{"type": "text", "text": text}]
                }),
                self.request_timeout,
            )
            .await
    }

    pub async fn shutdown(&self) -> ShutdownReport {
        self.connection.set_health(AdapterHealth::ShuttingDown);
        let (sender, receiver) = oneshot::channel();
        let supervisor_shutdown = async {
            if self
                .shutdown
                .send(ShutdownCommand { respond: sender })
                .await
                .is_ok()
            {
                receiver.await.unwrap_or(ShutdownReport {
                    reaped: false,
                    forced: true,
                })
            } else {
                self.final_report.borrow().unwrap_or(ShutdownReport {
                    reaped: false,
                    forced: true,
                })
            }
        };
        let close_writer =
            tokio::time::timeout(self.shutdown_grace, self.connection.close_writer());
        let supervisor_deadline = self.shutdown_grace.saturating_mul(2);
        let (report, _writer_closed) = tokio::join!(
            tokio::time::timeout(supervisor_deadline, supervisor_shutdown),
            close_writer
        );
        let report = report.unwrap_or(ShutdownReport {
            reaped: false,
            forced: true,
        });
        let mut tasks = self
            .tasks
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        let join_tasks = async {
            for task in &mut tasks {
                let _ignored = task.await;
            }
        };
        if tokio::time::timeout(self.shutdown_grace, join_tasks)
            .await
            .is_err()
        {
            for task in tasks {
                task.abort();
            }
        }
        report
    }

    async fn initialize(&self) -> Result<AcpCapabilities, AcpError> {
        let result = self
            .connection
            .request(
                "initialize",
                json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "clientCapabilities": {
                        "fs": {
                            "readTextFile": false,
                            "writeTextFile": false
                        },
                        "terminal": false
                    },
                    "clientInfo": {
                        "name": "workbench",
                        "title": "Workbench",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
                self.request_timeout,
            )
            .await?;
        let (capabilities, default_auth_method) = parse_initialize(&result)?;
        if let Some(method_id) = default_auth_method {
            self.connection
                .request(
                    "authenticate",
                    json!({"methodId": method_id}),
                    self.request_timeout,
                )
                .await?;
        }
        Ok(capabilities)
    }

    fn require_available(&self) -> Result<(), AcpError> {
        match self.capabilities.authentication {
            AuthenticationStatus::Available if self.health() == AdapterHealth::Available => Ok(()),
            AuthenticationStatus::InteractiveRequired => Err(AcpError::new(
                AcpErrorKind::AuthenticationRequired,
                "ACP provider authentication requires user interaction",
            )),
            _ => Err(AcpError::new(
                AcpErrorKind::TransportClosed,
                "ACP provider is unavailable",
            )),
        }
    }
}

impl Drop for GrokAcpClient {
    fn drop(&mut self) {
        let _ignored = self.shutdown.try_send(ShutdownCommand {
            respond: oneshot::channel().0,
        });
    }
}

fn canonical_executable(path: &Path) -> Result<PathBuf, AcpError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid_configuration())?;
    if !canonical.is_file() {
        return Err(invalid_configuration());
    }
    Ok(canonical)
}

fn canonical_workspace(path: &Path) -> Result<PathBuf, AcpError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid_configuration())?;
    if !canonical.is_dir() {
        return Err(invalid_configuration());
    }
    Ok(canonical)
}

fn spawn_stderr_drain(mut stderr: tokio::process::ChildStderr) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
}

fn spawn_supervisor(
    mut child: Child,
    mut shutdown: mpsc::Receiver<ShutdownCommand>,
    mut fatal: mpsc::Receiver<AcpError>,
    grace: Duration,
    connection: Arc<Connection>,
    final_report: watch::Sender<Option<ShutdownReport>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => {
                if status.is_err() {
                    connection.fail(&transport_closed(), AdapterHealth::Crashed);
                } else {
                    connection.fail(&transport_closed(), AdapterHealth::Unavailable);
                }
                shutdown.close();
                let report = ShutdownReport {
                    reaped: status.is_ok(),
                    forced: false,
                };
                let _ignored = final_report.send(Some(report));
                if let Some(command) = shutdown.recv().await {
                    let _ignored = command.respond.send(report);
                }
            }
            Some(command) = shutdown.recv() => {
                let (reaped, forced) = reap(&mut child, grace).await;
                let report = ShutdownReport { reaped, forced };
                let _ignored = final_report.send(Some(report));
                let _ignored = command.respond.send(report);
            }
            Some(_) = fatal.recv() => {
                let (reaped, forced) = force_reap(&mut child, grace).await;
                shutdown.close();
                let report = ShutdownReport { reaped, forced };
                let _ignored = final_report.send(Some(report));
                if let Some(command) = shutdown.recv().await {
                    let _ignored = command.respond.send(report);
                }
            }
        }
    })
}

async fn reap(child: &mut Child, grace: Duration) -> (bool, bool) {
    match tokio::time::timeout(grace, child.wait()).await {
        Ok(Ok(_)) => (true, false),
        Ok(Err(_)) => (false, false),
        Err(_) => force_reap(child, grace).await,
    }
}

async fn force_reap(child: &mut Child, grace: Duration) -> (bool, bool) {
    let _ignored = child.start_kill();
    (
        matches!(tokio::time::timeout(grace, child.wait()).await, Ok(Ok(_))),
        true,
    )
}

fn invalid_configuration() -> AcpError {
    AcpError::new(
        AcpErrorKind::InvalidConfiguration,
        "ACP launch configuration is invalid",
    )
}

fn spawn_failed() -> AcpError {
    AcpError::new(
        AcpErrorKind::SpawnFailed,
        "ACP provider could not be started",
    )
}

fn transport_closed() -> AcpError {
    AcpError::new(
        AcpErrorKind::TransportClosed,
        "ACP transport is unavailable",
    )
}
