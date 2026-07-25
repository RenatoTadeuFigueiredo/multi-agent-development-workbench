use std::{io, path::Path, sync::Arc, time::Duration};

use thiserror::Error;
use tokio::{sync::watch, task::JoinSet};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};
use workbench_config::ConfigError;
use workbench_mcp::McpGateway;
use workbench_storage::{PlatformKeyStore, SqliteStorage, StorageError};

use crate::{
    Application, BoundedTelemetry, ExternalTelemetryExport, FakeBehavior, TelemetryError,
    ipc::{BoundEndpoint, serve_connection},
    providers::{ProviderRuntime, ProviderRuntimeError},
    runtime_paths::{RuntimePathError, RuntimePaths, SingleDaemonLock},
    startup::StartupConfiguration,
};

#[derive(Clone)]
pub struct ShutdownHandle {
    sender: watch::Sender<bool>,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        let _ignored = self.sender.send(true);
    }
}

pub struct DaemonRuntime {
    application: Arc<Application>,
    endpoint: BoundEndpoint,
    _daemon_lock: SingleDaemonLock,
    shutdown_sender: watch::Sender<bool>,
    shutdown: watch::Receiver<bool>,
    providers: ProviderRuntime,
}

impl DaemonRuntime {
    /// Validates configuration and starts the persistent local daemon.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when paths, configuration, locks, encrypted
    /// storage, recovery, or endpoint binding fail.
    pub async fn start(
        paths: &RuntimePaths,
        repository_root: &Path,
    ) -> Result<(Self, ShutdownHandle), RuntimeError> {
        Self::start_with_configuration(paths, repository_root, None).await
    }

    /// Starts the daemon with an optional explicit highest-precedence
    /// configuration layer.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when paths, the explicit configuration, the
    /// matching repository lock, storage, recovery, or endpoint binding fail.
    pub async fn start_with_configuration(
        paths: &RuntimePaths,
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
    ) -> Result<(Self, ShutdownHandle), RuntimeError> {
        let owned_paths = paths.clone();
        let owned_repository_root = repository_root.to_path_buf();
        let owned_configuration = explicit_configuration.map(Path::to_path_buf);
        let (daemon_lock, startup, storage, telemetry, endpoint) =
            tokio::task::spawn_blocking(move || {
                owned_paths.prepare()?;
                let daemon_lock = SingleDaemonLock::acquire(&owned_paths)?;
                let startup = StartupConfiguration::load_with_configuration(
                    &owned_repository_root,
                    owned_configuration.as_deref(),
                )?;
                let storage =
                    SqliteStorage::open(&owned_paths.database_file, PlatformKeyStore::new())?;
                let telemetry = Arc::new(BoundedTelemetry::initialize(
                    ExternalTelemetryExport::Disabled,
                )?);
                let endpoint = BoundEndpoint::bind(&owned_paths, &daemon_lock)?;
                Ok::<_, RuntimeError>((daemon_lock, startup, storage, telemetry, endpoint))
            })
            .await
            .map_err(|_| RuntimeError::StartupTask)??;
        let spend_store: Option<std::sync::Arc<dyn workbench_openrouter::DurableSpendStore>> =
            Some(std::sync::Arc::new(crate::spend_store::PathSpendStore::new(
                &paths.database_file,
            )));
        let providers = ProviderRuntime::bootstrap_with_spend_store(
            &startup,
            repository_root,
            &paths.state_directory,
            spend_store,
        )
        .await?;
        let mcp_config = startup.resolved.clone();
        let mcp_lock = startup.base_lock.clone();
        let application = Application::new_with_providers_and_telemetry(
            storage,
            startup,
            FakeBehavior::default(),
            telemetry,
            providers.registry(),
            providers.catalog(),
        );
        // Central MCP gateway: pin verification only at attach; children spawn
        // on demand. Production uses the network HTTP/TLS client; tests inject
        // offline fakes via Application helpers.
        let workspace_key = paths
            .state_directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_owned();
        match McpGateway::bootstrap(
            mcp_config,
            &mcp_lock,
            paths.state_directory.join("mcp-runtime"),
            workspace_key,
            false,
        ) {
            Ok(gateway) => application.attach_mcp_gateway(std::sync::Arc::new(gateway)),
            Err(error) => {
                warn!(category = ?error.kind(), "MCP gateway unavailable at startup");
            }
        }
        if let Err(error) = application.recover() {
            providers.shutdown().await?;
            return Err(error.into());
        }
        let (sender, shutdown) = watch::channel(false);
        Ok((
            Self {
                application,
                endpoint,
                _daemon_lock: daemon_lock,
                shutdown_sender: sender.clone(),
                shutdown,
                providers,
            },
            ShutdownHandle { sender },
        ))
    }

    /// Accepts local clients until the shutdown handle is triggered.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when accepting clients or preparing durable
    /// shutdown fails.
    pub async fn run(mut self) -> Result<(), RuntimeError> {
        let serving = self.serve().await;
        let shutdown = self.application.prepare_shutdown().await;
        let providers_shutdown = self.providers.shutdown().await;
        shutdown?;
        providers_shutdown?;
        serving
    }

    async fn serve(&mut self) -> Result<(), RuntimeError> {
        let startup_deletions = self
            .application
            .run_maintenance(time::OffsetDateTime::now_utc())
            .await?;
        if startup_deletions > 0 {
            info!(
                retention_deletions = startup_deletions,
                "startup retention maintenance completed"
            );
        }
        let mut connections = JoinSet::new();
        let mut maintenance = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_mins(1),
            Duration::from_mins(1),
        );
        maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!("daemon accepting same-user local clients");
        loop {
            tokio::select! {
                changed = self.shutdown.changed() => {
                    if changed.is_err() || *self.shutdown.borrow() {
                        break;
                    }
                }
                accepted = self.endpoint.accept() => {
                    let (stream, _) = accepted?;
                    let application = Arc::clone(&self.application);
                    connections.spawn(async move {
                        if let Err(error) = serve_connection(application, stream).await {
                            warn!(error_kind = ?error.kind(), "client connection closed");
                        }
                    });
                }
                _ = maintenance.tick() => {
                    let deleted = self
                        .application
                        .run_maintenance(time::OffsetDateTime::now_utc())
                        .await?;
                    if deleted > 0 {
                        info!(retention_deletions = deleted, "retention maintenance completed");
                    }
                }
            }
        }
        let drain = async { while connections.join_next().await.is_some() {} };
        if tokio::time::timeout(Duration::from_secs(5), drain)
            .await
            .is_err()
        {
            connections.abort_all();
        }
        info!("daemon shutdown completed");
        Ok(())
    }

    /// Runs until completion or a process interrupt is received.
    ///
    /// # Errors
    ///
    /// Returns a runtime error from the daemon or signal handler.
    pub async fn run_until_signal(self) -> Result<(), RuntimeError> {
        let shutdown = ShutdownHandle {
            sender: self.shutdown_sender.clone(),
        };
        let run = self.run();
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => result,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                shutdown.shutdown();
                run.await
            }
        }
    }
}

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("workbench_daemon=info"));
    let _ignored = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(io::stderr)
                .with_current_span(false)
                .with_span_list(false),
        )
        .try_init();
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime paths are unsafe")]
    RuntimePath(#[from] RuntimePathError),
    #[error("configuration or lock validation failed")]
    Configuration(#[from] ConfigError),
    #[error("encrypted storage startup failed")]
    Storage(#[from] StorageError),
    #[error("telemetry initialization failed")]
    Telemetry(#[from] TelemetryError),
    #[error("provider preflight failed")]
    Provider(#[from] ProviderRuntimeError),
    #[error("local IPC failed")]
    Io(#[from] io::Error),
    #[error("daemon startup worker failed")]
    StartupTask,
}
