//! Provider adapter probing, registration, and supervised lifecycle.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use rustix::process::{Pid, Signal, getuid, kill_process_group, test_kill_process_group};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command};
use workbench_acp::{GrokLaunchProfile, GrokProviderAdapter};
use workbench_claude::{ClaudeLaunchProfile, ClaudeProviderAdapter};
use workbench_codex::{CodexLaunchProfile, CodexProviderAdapter};
use workbench_config::{
    ACP_PROTOCOL, AdapterInput, CLAUDE_CODE_STREAM_PROTOCOL, CODEX_EXEC_JSONL_PROTOCOL,
    ConfigError, OPENROUTER_CHAT_COMPLETIONS_PROTOCOL, canonicalize_adapter_executable,
    model::{ApprovalMode, Capability, EffectClass, ProviderDriver, ProviderType},
    preflight::{
        Authentication, ProviderCapabilities as ConfigProviderCapabilities, ProviderOperation,
    },
};
use workbench_core::{
    CoreError,
    ports::{
        AuthenticationStatus, ProviderAdapter, ProviderCapabilities, ProviderCapability,
        ProviderRegistry,
    },
    value::ProviderId,
};
use workbench_openrouter::{
    CostPolicyConfig, OpenRouterConnect, OpenRouterProviderAdapter, PlatformSecretSource,
    SecretSource, SessionCostLedger,
};

use crate::StartupConfiguration;

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_VERSION_OUTPUT_BYTES: usize = 4_096;
const MAX_VERSION_READ_BYTES: u64 = 4_097;
const AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_OUTPUT_BYTES: usize = 4_096;
const MAX_AUTH_READ_BYTES: u64 = 4_097;
const PROBE_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const CLAUDE_PROCESS_REAP_RESERVE: Duration = Duration::from_millis(450);
/// Leaves time inside the public cancellation deadline for the daemon to
/// persist and publish the terminal reconciliation outcome.
const CANCELLATION_FINALIZATION_RESERVE: Duration = Duration::from_millis(500);

/// Immutable adapter registry used by routing and dispatch.
#[derive(Default)]
pub struct StaticProviderRegistry {
    adapters: BTreeMap<ProviderId, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry for StaticProviderRegistry {
    fn adapter(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(provider).cloned()
    }
}

/// Workspace-owned provider processes and their preflight catalog.
pub struct ProviderRuntime {
    registry: Arc<StaticProviderRegistry>,
    catalog: BTreeMap<String, ConfigProviderCapabilities>,
    managed: Vec<ManagedAdapter>,
    openrouter_ledger: SessionCostLedger,
    _snapshots: TempDir,
}

struct ProviderDescriptor {
    kind: AdapterProbeKind,
    provider_id: ProviderId,
    version: String,
    protocol: String,
    executable: PathBuf,
}

#[derive(Clone)]
enum ManagedAdapter {
    Acp(Arc<GrokProviderAdapter>),
    ClaudeCode(Arc<ClaudeProviderAdapter>),
    Codex(Arc<CodexProviderAdapter>),
    OpenRouter(Arc<OpenRouterProviderAdapter>),
}

/// Executable protocol selected explicitly by resolved provider
/// configuration before a lock probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterProbeKind {
    Acp,
    ClaudeCode,
    Codex,
}

/// Canonical executable and driver used for explicit lock regeneration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterProbe {
    pub kind: AdapterProbeKind,
    pub executable: PathBuf,
}

impl AdapterProbe {
    #[must_use]
    pub fn acp(executable: PathBuf) -> Self {
        Self {
            kind: AdapterProbeKind::Acp,
            executable,
        }
    }

    #[must_use]
    pub fn claude_code(executable: PathBuf) -> Self {
        Self {
            kind: AdapterProbeKind::ClaudeCode,
            executable,
        }
    }

    #[must_use]
    pub fn codex(executable: PathBuf) -> Self {
        Self {
            kind: AdapterProbeKind::Codex,
            executable,
        }
    }
}

impl ProviderRuntime {
    /// Starts every configured ACP adapter after the repository lock has
    /// already passed the static digest check.
    ///
    /// # Errors
    ///
    /// Returns a redacted provider error when an adapter cannot initialize or
    /// its reported identity differs from the committed pin.
    pub async fn bootstrap(
        startup: &StartupConfiguration,
        workspace: &Path,
        snapshot_root: &Path,
    ) -> Result<Self, ProviderRuntimeError> {
        if !startup.lock_is_verified() {
            return Err(ProviderRuntimeError::Incompatible(
                "provider startup requires a verified repository lock",
            ));
        }
        let snapshot_root_metadata =
            fs::symlink_metadata(snapshot_root).map_err(|_| ProviderRuntimeError::Snapshot)?;
        if snapshot_root_metadata.file_type().is_symlink()
            || !snapshot_root_metadata.is_dir()
            || snapshot_root_metadata.uid() != getuid().as_raw()
            || snapshot_root_metadata.permissions().mode() & 0o022 != 0
        {
            return Err(ProviderRuntimeError::Snapshot);
        }
        let snapshots = tempfile::Builder::new()
            .prefix(".provider-snapshots-")
            .tempdir_in(snapshot_root)
            .map_err(|_| ProviderRuntimeError::Snapshot)?;
        let owned_startup = startup.clone();
        let snapshot_directory = snapshots.path().to_path_buf();
        let descriptors = tokio::task::spawn_blocking(move || {
            provider_descriptors(&owned_startup, &snapshot_directory)
        })
        .await
        .map_err(|_| ProviderRuntimeError::Task)??;
        let mut adapters = BTreeMap::new();
        let mut catalog = BTreeMap::new();
        let mut managed = Vec::new();
        let cancellation_deadline = provider_cancellation_budget(Duration::from_millis(
            startup.resolved.protocol.cancellation_deadline_ms,
        ))?;
        let openrouter_ledger = SessionCostLedger::new();
        let secrets: Arc<dyn SecretSource> = Arc::new(PlatformSecretSource::new());

        for (name, descriptor) in descriptors {
            let (adapter, owned) =
                match connect_descriptor(&descriptor, workspace, cancellation_deadline).await {
                    Ok(connected) => connected,
                    Err(error) => {
                        cleanup_after_startup_error(&managed, &error).await?;
                        return Err(error.into());
                    }
                };
            let capabilities = match adapter.capabilities().await {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    let mut cleanup = managed.clone();
                    cleanup.push(owned);
                    if !shutdown_managed(&cleanup).await {
                        return Err(ProviderRuntimeError::Reap);
                    }
                    return Err(error.into());
                }
            };
            if capabilities.adapter_version != descriptor.version
                || capabilities.protocol != descriptor.protocol
            {
                let mut cleanup = managed.clone();
                cleanup.push(owned);
                if !shutdown_managed(&cleanup).await {
                    return Err(ProviderRuntimeError::Reap);
                }
                return Err(ProviderRuntimeError::Incompatible(
                    "provider adapter identity differs from the lock",
                ));
            }
            catalog.insert(name.clone(), config_capabilities(&capabilities));
            adapters.insert(descriptor.provider_id, adapter);
            managed.push(owned);
        }

        match connect_openrouter_providers(
            startup,
            Arc::clone(&secrets),
            openrouter_ledger.clone(),
            cancellation_deadline,
        ) {
            Ok(api_adapters) => {
                for (name, provider_id, adapter, owned) in api_adapters {
                    let capabilities = match adapter.capabilities().await {
                        Ok(capabilities) => capabilities,
                        Err(error) => {
                            let mut cleanup = managed.clone();
                            cleanup.push(owned);
                            if !shutdown_managed(&cleanup).await {
                                return Err(ProviderRuntimeError::Reap);
                            }
                            return Err(error.into());
                        }
                    };
                    catalog.insert(name, config_capabilities(&capabilities));
                    adapters.insert(provider_id, adapter);
                    managed.push(owned);
                }
            }
            Err(error) => {
                cleanup_after_startup_error(&managed, &error).await?;
                return Err(error.into());
            }
        }

        Ok(Self {
            registry: Arc::new(StaticProviderRegistry { adapters }),
            catalog,
            managed,
            openrouter_ledger,
            _snapshots: snapshots,
        })
    }

    /// Session cost ledger shared by OpenRouter adapters in this runtime.
    #[must_use]
    pub fn openrouter_ledger(&self) -> &SessionCostLedger {
        &self.openrouter_ledger
    }

    /// Boots only OpenRouter API adapters for offline acceptance tests.
    ///
    /// # Errors
    ///
    /// Returns when configuration or credential policy is invalid.
    pub fn bootstrap_openrouter_only(
        startup: &StartupConfiguration,
        secrets: Arc<dyn SecretSource>,
        ledger: SessionCostLedger,
    ) -> Result<Self, ProviderRuntimeError> {
        if !startup.lock_is_verified() {
            return Err(ProviderRuntimeError::Incompatible(
                "provider startup requires a verified repository lock",
            ));
        }
        let cancellation_deadline = provider_cancellation_budget(Duration::from_millis(
            startup.resolved.protocol.cancellation_deadline_ms,
        ))?;
        let snapshots = tempfile::Builder::new()
            .prefix(".provider-snapshots-")
            .tempdir()
            .map_err(|_| ProviderRuntimeError::Snapshot)?;
        let mut adapters = BTreeMap::new();
        let mut catalog = BTreeMap::new();
        let mut managed = Vec::new();
        let api_adapters =
            connect_openrouter_providers(startup, secrets, ledger.clone(), cancellation_deadline)?;
        for (name, provider_id, adapter, owned) in api_adapters {
            // capabilities() is async; use blocking via known offline values.
            catalog.insert(
                name.clone(),
                ConfigProviderCapabilities {
                    adapter_id: name,
                    protocol: OPENROUTER_CHAT_COMPLETIONS_PROTOCOL.to_owned(),
                    adapter_version: "1".to_owned(),
                    authentication: Authentication::Available,
                    capabilities: vec![Capability::Streaming, Capability::Cancellation],
                    operations: vec![ProviderOperation {
                        name: "provider.prompt".to_owned(),
                        effect_class: EffectClass::PaidInference,
                        idempotent: false,
                        material_cost: true,
                        approval: ApprovalMode::Policy,
                    }],
                    context_window_tokens: Some(128_000),
                },
            );
            adapters.insert(provider_id, adapter);
            managed.push(owned);
        }
        Ok(Self {
            registry: Arc::new(StaticProviderRegistry { adapters }),
            catalog,
            managed,
            openrouter_ledger: ledger,
            _snapshots: snapshots,
        })
    }

    #[must_use]
    pub fn registry(&self) -> Arc<dyn ProviderRegistry> {
        self.registry.clone()
    }

    #[must_use]
    pub fn catalog(&self) -> BTreeMap<String, ConfigProviderCapabilities> {
        self.catalog.clone()
    }

    /// Stops and reaps every workspace-owned provider child.
    ///
    /// # Errors
    ///
    /// Returns an error when any supervised child cannot be reaped within the
    /// bounded shutdown deadline.
    pub async fn shutdown(&self) -> Result<(), ProviderRuntimeError> {
        if shutdown_managed(&self.managed).await {
            Ok(())
        } else {
            Err(ProviderRuntimeError::Reap)
        }
    }
}

async fn connect_descriptor(
    descriptor: &ProviderDescriptor,
    workspace: &Path,
    cancellation_deadline: Duration,
) -> Result<(Arc<dyn ProviderAdapter>, ManagedAdapter), CoreError> {
    match descriptor.kind {
        AdapterProbeKind::Acp => {
            let adapter = Arc::new(
                GrokProviderAdapter::connect(
                    descriptor.provider_id.clone(),
                    descriptor.version.clone(),
                    GrokLaunchProfile::new(&descriptor.executable, workspace),
                    cancellation_deadline,
                )
                .await?,
            );
            let erased: Arc<dyn ProviderAdapter> = adapter.clone();
            Ok((erased, ManagedAdapter::Acp(adapter)))
        }
        AdapterProbeKind::ClaudeCode => {
            let initialization_timeout = cancellation_deadline
                .checked_sub(CLAUDE_PROCESS_REAP_RESERVE)
                .filter(|timeout| !timeout.is_zero())
                .unwrap_or(cancellation_deadline / 2);
            let adapter = Arc::new(
                ClaudeProviderAdapter::connect(
                    descriptor.provider_id.clone(),
                    descriptor.version.clone(),
                    ClaudeLaunchProfile::new(&descriptor.executable, workspace)
                        .initialization_timeout(initialization_timeout),
                    cancellation_deadline,
                )
                .await?,
            );
            let erased: Arc<dyn ProviderAdapter> = adapter.clone();
            Ok((erased, ManagedAdapter::ClaudeCode(adapter)))
        }
        AdapterProbeKind::Codex => {
            let preflight_timeout = cancellation_deadline
                .checked_sub(CLAUDE_PROCESS_REAP_RESERVE)
                .filter(|timeout| !timeout.is_zero())
                .unwrap_or(cancellation_deadline / 2);
            let adapter = Arc::new(
                CodexProviderAdapter::connect(
                    descriptor.provider_id.clone(),
                    descriptor.version.clone(),
                    CodexLaunchProfile::new(&descriptor.executable, workspace)
                        .preflight_timeout(preflight_timeout),
                    cancellation_deadline,
                )
                .await?,
            );
            let erased: Arc<dyn ProviderAdapter> = adapter.clone();
            Ok((erased, ManagedAdapter::Codex(adapter)))
        }
    }
}

fn connect_openrouter_providers(
    startup: &StartupConfiguration,
    secrets: Arc<dyn SecretSource>,
    ledger: SessionCostLedger,
    cancellation_deadline: Duration,
) -> Result<
    Vec<(
        String,
        ProviderId,
        Arc<dyn ProviderAdapter>,
        ManagedAdapter,
    )>,
    CoreError,
> {
    let mut connected = Vec::new();
    let cost = startup.resolved.policies.cost.as_ref();
    for (name, provider) in &startup.resolved.providers {
        if provider.kind != ProviderType::Api {
            continue;
        }
        let Some(cost) = cost else {
            return Err(CoreError::new(
                workbench_core::FailureCategory::InvalidRequest,
                "API providers require policies.cost",
            ));
        };
        let credential_ref = provider.credential_ref.clone().ok_or_else(|| {
            CoreError::new(
                workbench_core::FailureCategory::InvalidRequest,
                "API provider is missing credential_ref",
            )
        })?;
        let zero_data_retention = provider
            .privacy
            .as_ref()
            .is_some_and(|privacy| privacy.zero_data_retention);
        let provider_id = ProviderId::parse(name.clone())?;
        let transport =
            OpenRouterProviderAdapter::transport_for_base_url(provider.base_url.as_deref());
        let adapter = Arc::new(OpenRouterProviderAdapter::connect(OpenRouterConnect {
            adapter_id: provider_id.clone(),
            adapter_version: "1".to_owned(),
            credential_ref,
            secrets: Arc::clone(&secrets),
            transport,
            ledger: ledger.clone(),
            policy: CostPolicyConfig {
                max_session_usd_micros: cost.max_session_usd_micros,
                max_attempt_usd_micros: cost.max_attempt_usd_micros,
            },
            zero_data_retention,
            cancellation_deadline,
            // Secrets may be installed after lock generation; fail at prompt.
            require_secret_at_connect: false,
        })?);
        let erased: Arc<dyn ProviderAdapter> = adapter.clone();
        connected.push((
            name.clone(),
            provider_id,
            erased,
            ManagedAdapter::OpenRouter(adapter),
        ));
    }
    Ok(connected)
}

fn provider_cancellation_budget(
    public_deadline: Duration,
) -> Result<Duration, ProviderRuntimeError> {
    public_deadline
        .checked_sub(CANCELLATION_FINALIZATION_RESERVE)
        .filter(|budget| !budget.is_zero())
        .ok_or(ProviderRuntimeError::Incompatible(
            "cancellation deadline leaves no finalization budget",
        ))
}

fn provider_descriptors(
    startup: &StartupConfiguration,
    snapshot_directory: &Path,
) -> Result<BTreeMap<String, ProviderDescriptor>, ProviderRuntimeError> {
    let mut descriptors = BTreeMap::new();
    for (name, provider) in &startup.resolved.providers {
        let kind = match (provider.kind, provider.driver) {
            (ProviderType::Acp, None) => AdapterProbeKind::Acp,
            (ProviderType::SubscriptionCli, Some(ProviderDriver::ClaudeCode)) => {
                AdapterProbeKind::ClaudeCode
            }
            (ProviderType::SubscriptionCli, Some(ProviderDriver::Codex)) => AdapterProbeKind::Codex,
            _ => continue,
        };
        let provider_id = ProviderId::parse(name.clone())?;
        let executable =
            provider
                .executable
                .as_deref()
                .ok_or(ProviderRuntimeError::Incompatible(
                    "provider executable is not configured",
                ))?;
        let locked =
            startup
                .base_lock
                .adapters
                .get(name)
                .ok_or(ProviderRuntimeError::Incompatible(
                    "provider adapter is absent from the lock",
                ))?;
        let executable = snapshot_locked_executable(
            kind,
            name,
            Path::new(executable),
            &locked.executable_sha256,
            snapshot_directory,
        )?;
        descriptors.insert(
            name.clone(),
            ProviderDescriptor {
                kind,
                provider_id,
                version: locked.version.clone(),
                protocol: locked.protocol.clone(),
                executable,
            },
        );
    }
    Ok(descriptors)
}

fn snapshot_locked_executable(
    kind: AdapterProbeKind,
    name: &str,
    source: &Path,
    expected_sha256: &str,
    snapshot_directory: &Path,
) -> Result<PathBuf, ProviderRuntimeError> {
    let (target, digest) = copy_executable_snapshot(name, source, snapshot_directory)?;
    if digest != expected_sha256 {
        return Err(ProviderRuntimeError::Incompatible(match kind {
            AdapterProbeKind::Acp => "ACP executable differs from the lock",
            AdapterProbeKind::ClaudeCode => "Claude Code executable differs from the lock",
            AdapterProbeKind::Codex => "Codex executable differs from the lock",
        }));
    }
    Ok(target)
}

fn copy_executable_snapshot(
    name: &str,
    source: &Path,
    snapshot_directory: &Path,
) -> Result<(PathBuf, String), ProviderRuntimeError> {
    let source = canonicalize_adapter_executable(source)?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(source)
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    let input_metadata = input
        .metadata()
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    if !input_metadata.is_file()
        || input_metadata.uid() != getuid().as_raw()
        || input_metadata.permissions().mode() & 0o022 != 0
        || input_metadata.permissions().mode() & 0o111 == 0
    {
        return Err(ProviderRuntimeError::Snapshot);
    }
    let target = snapshot_directory.join(name);
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&target)
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| ProviderRuntimeError::Snapshot)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|_| ProviderRuntimeError::Snapshot)?;
    }
    let digest = hex::encode(digest.finalize());
    output
        .sync_all()
        .and_then(|()| fs::set_permissions(&target, fs::Permissions::from_mode(0o500)))
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    Ok((target, digest))
}

/// Probes all configured ACP executable versions for an explicit lock
/// regeneration.
///
/// # Errors
///
/// Returns before a lock is written when an executable is unsafe, hangs,
/// exits unsuccessfully, or reports a malformed version.
pub async fn probe_adapter_inputs(
    executables: &BTreeMap<String, PathBuf>,
    workspace: &Path,
) -> Result<BTreeMap<String, AdapterInput>, ProviderRuntimeError> {
    let probes = executables
        .iter()
        .map(|(name, executable)| (name.clone(), AdapterProbe::acp(executable.clone())))
        .collect();
    probe_configured_adapter_inputs(&probes, workspace).await
}

/// Probes explicitly configured adapter executables for lock regeneration.
///
/// # Errors
///
/// Returns a redacted error when a probe target is unsafe, incompatible,
/// unauthenticated, oversized, malformed, or cannot be reaped.
pub async fn probe_configured_adapter_inputs(
    probes: &BTreeMap<String, AdapterProbe>,
    workspace: &Path,
) -> Result<BTreeMap<String, AdapterInput>, ProviderRuntimeError> {
    let validated = probes
        .iter()
        .map(|(name, probe)| {
            Ok((
                name.clone(),
                AdapterProbe {
                    kind: probe.kind,
                    executable: canonicalize_adapter_executable(&probe.executable)?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
    let snapshots = tempfile::Builder::new()
        .prefix(".adapter-probe-")
        .tempdir_in(workspace)
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    let mut inputs = BTreeMap::new();
    for (name, probe) in validated {
        let (snapshot, executable_sha256) =
            copy_executable_snapshot(&name, &probe.executable, snapshots.path())?;
        let (protocol, version) = match probe.kind {
            AdapterProbeKind::Acp => (
                ACP_PROTOCOL,
                probe_grok_version(&snapshot, workspace).await?,
            ),
            AdapterProbeKind::ClaudeCode => {
                let version = probe_claude_version(&snapshot, workspace).await?;
                probe_claude_subscription_auth(&snapshot, workspace).await?;
                (CLAUDE_CODE_STREAM_PROTOCOL, version)
            }
            AdapterProbeKind::Codex => {
                let version = probe_codex_version(&snapshot, workspace).await?;
                probe_codex_subscription_auth(&snapshot, workspace).await?;
                (CODEX_EXEC_JSONL_PROTOCOL, version)
            }
        };
        inputs.insert(
            name,
            AdapterInput {
                protocol: protocol.to_owned(),
                version,
                executable: probe.executable,
                executable_sha256,
            },
        );
    }
    Ok(inputs)
}

async fn probe_claude_version(
    executable: &Path,
    workspace: &Path,
) -> Result<String, ProviderRuntimeError> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .env("DISABLE_AUTOUPDATER", "1")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_claude_billing_environment(&mut command);
    let bytes = run_bounded_probe(
        command,
        VERSION_PROBE_TIMEOUT,
        MAX_VERSION_READ_BYTES,
        MAX_VERSION_OUTPUT_BYTES,
    )
    .await?;
    normalize_claude_version(&bytes)
}

async fn probe_claude_subscription_auth(
    executable: &Path,
    workspace: &Path,
) -> Result<(), ProviderRuntimeError> {
    let mut command = Command::new(executable);
    command
        .args(["auth", "status", "--json"])
        .env("DISABLE_AUTOUPDATER", "1")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_claude_billing_environment(&mut command);
    let bytes = run_bounded_probe(
        command,
        AUTH_PROBE_TIMEOUT,
        MAX_AUTH_READ_BYTES,
        MAX_AUTH_OUTPUT_BYTES,
    )
    .await?;
    validate_claude_subscription_auth(&bytes)
}

async fn probe_codex_version(
    executable: &Path,
    workspace: &Path,
) -> Result<String, ProviderRuntimeError> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_codex_billing_environment(&mut command);
    let bytes = run_bounded_probe(
        command,
        VERSION_PROBE_TIMEOUT,
        MAX_VERSION_READ_BYTES,
        MAX_VERSION_OUTPUT_BYTES,
    )
    .await?;
    normalize_codex_version(&bytes)
}

async fn probe_codex_subscription_auth(
    executable: &Path,
    workspace: &Path,
) -> Result<(), ProviderRuntimeError> {
    let mut command = Command::new(executable);
    command
        .args(["login", "status"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_codex_billing_environment(&mut command);
    let bytes = run_bounded_probe(
        command,
        AUTH_PROBE_TIMEOUT,
        MAX_AUTH_READ_BYTES,
        MAX_AUTH_OUTPUT_BYTES,
    )
    .await?;
    validate_codex_subscription_auth(&bytes)
}

async fn run_bounded_probe(
    mut command: Command,
    timeout: Duration,
    max_read_bytes: u64,
    max_output_bytes: usize,
) -> Result<Vec<u8>, ProviderRuntimeError> {
    let mut child = command
        .spawn()
        .map_err(|_| ProviderRuntimeError::Probe("adapter probe failed"))?;
    let process_group = child
        .id()
        .and_then(|raw_pid| Pid::from_raw(raw_pid.cast_signed()));
    let Some(stdout) = child.stdout.take() else {
        if !reap_failed_probe_group(&mut child, process_group).await {
            return Err(ProviderRuntimeError::Reap);
        }
        return Err(ProviderRuntimeError::Probe("adapter probe failed"));
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(max_read_bytes)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    };
    let bytes = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(bytes)) if bytes.len() <= max_output_bytes => bytes,
        _ => {
            if !reap_failed_probe_group(&mut child, process_group).await {
                return Err(ProviderRuntimeError::Reap);
            }
            return Err(ProviderRuntimeError::Probe("adapter probe failed"));
        }
    };
    let Ok(Ok(status)) = tokio::time::timeout(timeout, child.wait()).await else {
        if !reap_failed_probe_group(&mut child, process_group).await {
            return Err(ProviderRuntimeError::Reap);
        }
        return Err(ProviderRuntimeError::Probe("adapter probe failed"));
    };
    let group_state = cleanup_remaining_probe_group(process_group).await;
    if group_state == ProbeGroupState::ReapFailed {
        return Err(ProviderRuntimeError::Reap);
    }
    if !status.success() {
        return Err(ProviderRuntimeError::Probe("adapter probe failed"));
    }
    if group_state == ProbeGroupState::ResidualReaped {
        return Err(ProviderRuntimeError::Probe(
            "adapter probe spawned an unexpected child",
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeGroupState {
    Clean,
    ResidualReaped,
    ReapFailed,
}

async fn cleanup_remaining_probe_group(process_group: Option<Pid>) -> ProbeGroupState {
    let Some(process_group) = process_group else {
        return ProbeGroupState::ReapFailed;
    };
    if test_kill_process_group(process_group).is_err() {
        return ProbeGroupState::Clean;
    }
    let _ignored = kill_process_group(process_group, Signal::TERM);
    if wait_for_probe_group_exit(process_group).await {
        return ProbeGroupState::ResidualReaped;
    }
    let _ignored = kill_process_group(process_group, Signal::KILL);
    if wait_for_probe_group_exit(process_group).await {
        ProbeGroupState::ResidualReaped
    } else {
        ProbeGroupState::ReapFailed
    }
}

async fn wait_for_probe_group_exit(process_group: Pid) -> bool {
    let exited = async {
        while test_kill_process_group(process_group).is_ok() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(PROBE_SHUTDOWN_GRACE, exited)
        .await
        .is_ok()
}

async fn reap_failed_probe_group(
    child: &mut tokio::process::Child,
    process_group: Option<Pid>,
) -> bool {
    if let Some(process_group) = process_group {
        let _ignored = kill_process_group(process_group, Signal::KILL);
    }
    let _ignored = child.start_kill();
    let child_reaped = matches!(
        tokio::time::timeout(VERSION_PROBE_TIMEOUT, child.wait()).await,
        Ok(Ok(_))
    );
    child_reaped
        && cleanup_remaining_probe_group(process_group).await != ProbeGroupState::ReapFailed
}

fn normalize_claude_version(bytes: &[u8]) -> Result<String, ProviderRuntimeError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| ProviderRuntimeError::Probe("Claude Code version probe failed"))?
        .trim();
    let version = output
        .strip_suffix(" (Claude Code)")
        .unwrap_or(output)
        .trim();
    let mut components = version.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let minor = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let patch = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    if components.next().is_some()
        || !matches!((major, minor, patch), (Some(_), Some(_), Some(_)))
        || version.len() > 255
        || version.chars().any(char::is_control)
    {
        return Err(ProviderRuntimeError::Probe(
            "Claude Code version probe failed",
        ));
    }
    if (
        major.unwrap_or_default(),
        minor.unwrap_or_default(),
        patch.unwrap_or_default(),
    ) < (2, 1, 214)
    {
        return Err(ProviderRuntimeError::Incompatible(
            "Claude Code 2.1.214 or newer is required",
        ));
    }
    Ok(version.to_owned())
}

fn validate_claude_subscription_auth(bytes: &[u8]) -> Result<(), ProviderRuntimeError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthStatus {
        logged_in: bool,
        auth_method: String,
        api_provider: String,
    }

    let status: AuthStatus = serde_json::from_slice(bytes)
        .map_err(|_| ProviderRuntimeError::Probe("Claude Code auth probe failed"))?;
    let subscription = matches!(status.auth_method.as_str(), "claude.ai" | "claudeai");
    if !status.logged_in
        || !subscription
        || status.api_provider != "firstParty"
        || status.auth_method.len() > 64
        || status.api_provider.len() > 64
    {
        return Err(ProviderRuntimeError::Incompatible(
            "Claude Code subscription authentication is unavailable",
        ));
    }
    Ok(())
}

pub(crate) fn sanitize_claude_billing_environment(command: &mut Command) {
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "ANTHROPIC_BEDROCK_BASE_URL",
        "ANTHROPIC_VERTEX_BASE_URL",
        "ANTHROPIC_FOUNDRY_BASE_URL",
    ] {
        command.env_remove(name);
    }
}

pub(crate) fn sanitize_codex_billing_environment(command: &mut Command) {
    for name in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_API_BASE",
        "OPENAI_ORG_ID",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT",
        "CODEX_OSS_BASE_URL",
        "OLLAMA_BASE_URL",
        "OPENAI_API_KEY_PATH",
    ] {
        command.env_remove(name);
    }
}

fn normalize_codex_version(bytes: &[u8]) -> Result<String, ProviderRuntimeError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| ProviderRuntimeError::Probe("Codex version probe failed"))?
        .trim();
    if output.lines().count() != 1 {
        return Err(ProviderRuntimeError::Probe("Codex version probe failed"));
    }
    let version = output
        .strip_prefix("codex-cli ")
        .or_else(|| output.strip_prefix("codex "))
        .unwrap_or(output)
        .trim();
    let mut components = version.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let minor = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let patch = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    if components.next().is_some()
        || !matches!((major, minor, patch), (Some(_), Some(_), Some(_)))
        || version.len() > 255
        || version.chars().any(char::is_control)
    {
        return Err(ProviderRuntimeError::Probe("Codex version probe failed"));
    }
    if (
        major.unwrap_or_default(),
        minor.unwrap_or_default(),
        patch.unwrap_or_default(),
    ) < (0, 145, 0)
    {
        return Err(ProviderRuntimeError::Incompatible(
            "Codex CLI 0.145.0 or newer is required",
        ));
    }
    Ok(version.to_owned())
}

fn validate_codex_subscription_auth(bytes: &[u8]) -> Result<(), ProviderRuntimeError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| ProviderRuntimeError::Probe("Codex auth probe failed"))?
        .trim();
    if output.lines().count() > 8 || output.len() > MAX_AUTH_OUTPUT_BYTES {
        return Err(ProviderRuntimeError::Probe("Codex auth probe failed"));
    }
    let normalized = output.to_ascii_lowercase();
    if normalized.contains("logged in using chatgpt")
        && !normalized.contains("api key")
        && !normalized.contains("api-key")
    {
        Ok(())
    } else {
        Err(ProviderRuntimeError::Incompatible(
            "Codex subscription authentication is unavailable",
        ))
    }
}

async fn probe_grok_version(
    executable: &Path,
    workspace: &Path,
) -> Result<String, ProviderRuntimeError> {
    let executable = std::fs::canonicalize(executable)
        .map_err(|_| ProviderRuntimeError::Probe("adapter version probe failed"))?;
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|_| ProviderRuntimeError::Probe("adapter version probe failed"))?;
    let mut child = Command::new(executable)
        .arg("--version")
        .env("GROK_DISABLE_AUTOUPDATER", "1")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ProviderRuntimeError::Probe("adapter version probe failed"))?;
    let Some(stdout) = child.stdout.take() else {
        if !reap_failed_probe(&mut child).await {
            return Err(ProviderRuntimeError::Reap);
        }
        return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_VERSION_READ_BYTES)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    };
    let bytes = match tokio::time::timeout(VERSION_PROBE_TIMEOUT, read).await {
        Ok(Ok(bytes)) if bytes.len() <= MAX_VERSION_OUTPUT_BYTES => bytes,
        _ => {
            if !reap_failed_probe(&mut child).await {
                return Err(ProviderRuntimeError::Reap);
            }
            return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
        }
    };
    let Ok(Ok(status)) = tokio::time::timeout(VERSION_PROBE_TIMEOUT, child.wait()).await else {
        if !reap_failed_probe(&mut child).await {
            return Err(ProviderRuntimeError::Reap);
        }
        return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
    };
    if !status.success() {
        return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
    }
    normalize_grok_version(&bytes)
}

fn normalize_grok_version(bytes: &[u8]) -> Result<String, ProviderRuntimeError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| ProviderRuntimeError::Probe("adapter version probe failed"))?
        .trim();
    if output.lines().count() != 1 {
        return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
    }
    let version = output
        .strip_prefix("grok ")
        .ok_or(ProviderRuntimeError::Probe("adapter version probe failed"))?;
    let version = version
        .rfind(" [")
        .filter(|index| version.ends_with(']') && *index > 0)
        .map_or(version, |index| &version[..index]);
    if version.is_empty() || version.len() > 255 || version.chars().any(char::is_control) {
        return Err(ProviderRuntimeError::Probe("adapter version probe failed"));
    }
    Ok(version.to_owned())
}

async fn reap_failed_probe(child: &mut tokio::process::Child) -> bool {
    let _ignored = child.start_kill();
    matches!(
        tokio::time::timeout(VERSION_PROBE_TIMEOUT, child.wait()).await,
        Ok(Ok(_))
    )
}

async fn cleanup_after_startup_error(
    managed: &[ManagedAdapter],
    error: &CoreError,
) -> Result<(), ProviderRuntimeError> {
    if !shutdown_managed(managed).await
        || error.category() == workbench_core::FailureCategory::Internal
    {
        Err(ProviderRuntimeError::Reap)
    } else {
        Ok(())
    }
}

async fn shutdown_managed(adapters: &[ManagedAdapter]) -> bool {
    futures_util::future::join_all(adapters.iter().rev().map(|adapter| async move {
        match adapter {
            ManagedAdapter::Acp(adapter) => adapter.shutdown().await.reaped,
            ManagedAdapter::ClaudeCode(adapter) => adapter.shutdown().await.reaped,
            ManagedAdapter::Codex(adapter) => adapter.shutdown().await.reaped,
            ManagedAdapter::OpenRouter(adapter) => {
                adapter.shutdown();
                true
            }
        }
    }))
    .await
    .into_iter()
    .all(|reaped| reaped)
}

fn config_capabilities(capabilities: &ProviderCapabilities) -> ConfigProviderCapabilities {
    ConfigProviderCapabilities {
        adapter_id: capabilities.adapter_id.as_str().to_owned(),
        adapter_version: capabilities.adapter_version.clone(),
        protocol: capabilities.protocol.clone(),
        authentication: match capabilities.authentication {
            AuthenticationStatus::Available => Authentication::Available,
            AuthenticationStatus::Unavailable => Authentication::Unavailable,
            AuthenticationStatus::Expired => Authentication::Expired,
            AuthenticationStatus::InteractiveRequired => Authentication::InteractiveRequired,
        },
        capabilities: capabilities
            .capabilities
            .iter()
            .map(|capability| match capability {
                ProviderCapability::Streaming => Capability::Streaming,
                ProviderCapability::ToolCalling => Capability::ToolCalling,
                ProviderCapability::StructuredOutput => Capability::StructuredOutput,
                ProviderCapability::SessionResume => Capability::SessionResume,
                ProviderCapability::Cancellation => Capability::Cancellation,
                ProviderCapability::Vision => Capability::Vision,
                ProviderCapability::Mcp => Capability::Mcp,
                ProviderCapability::Acp => Capability::Acp,
            })
            .collect(),
        context_window_tokens: capabilities.context_window_tokens,
        operations: vec![ProviderOperation {
            name: "provider.prompt".to_owned(),
            effect_class: EffectClass::PaidInference,
            idempotent: false,
            material_cost: true,
            approval: ApprovalMode::Policy,
        }],
    }
}

#[derive(Debug, Error)]
pub enum ProviderRuntimeError {
    #[error("provider configuration is invalid")]
    Configuration(#[from] ConfigError),
    #[error("provider preflight failed")]
    Core(#[from] CoreError),
    #[error("{0}")]
    Incompatible(&'static str),
    #[error("{0}")]
    Probe(&'static str),
    #[error("provider executable snapshot failed")]
    Snapshot,
    #[error("provider process could not be reaped")]
    Reap,
    #[error("provider startup worker failed")]
    Task,
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};

    use tempfile::TempDir;
    use tokio::process::Command;

    use sha2::{Digest, Sha256};

    use super::{
        AdapterProbe, AdapterProbeKind, CANCELLATION_FINALIZATION_RESERVE, ProviderRuntime,
        normalize_claude_version, normalize_codex_version, normalize_grok_version,
        probe_adapter_inputs, probe_claude_version, probe_configured_adapter_inputs,
        provider_cancellation_budget, sanitize_claude_billing_environment,
        sanitize_codex_billing_environment, snapshot_locked_executable,
        validate_claude_subscription_auth, validate_codex_subscription_auth,
    };
    use crate::StartupConfiguration;

    #[test]
    fn normalizes_grok_version_without_the_mutable_channel_label() {
        assert_eq!(
            normalize_grok_version(b"grok 0.2.7 (95d84f) [stable]\n").expect("version"),
            "0.2.7 (95d84f)"
        );
        assert_eq!(
            normalize_grok_version(b"grok 0.2.7-test\n").expect("version"),
            "0.2.7-test"
        );
    }

    #[test]
    fn rejects_multiline_or_unbounded_version_output() {
        assert!(normalize_grok_version(b"grok 1\ngrok 2\n").is_err());
        assert!(normalize_grok_version(format!("grok {}", "a".repeat(256)).as_bytes()).is_err());
    }

    #[test]
    fn provider_cancellation_budget_preserves_the_public_deadline() {
        let public_deadline = Duration::from_secs(5);
        let provider_budget =
            provider_cancellation_budget(public_deadline).expect("provider cancellation budget");
        assert_eq!(provider_budget, Duration::from_millis(4_500));
        assert_eq!(
            provider_budget + CANCELLATION_FINALIZATION_RESERVE,
            public_deadline
        );
        assert!(
            provider_cancellation_budget(CANCELLATION_FINALIZATION_RESERVE).is_err(),
            "the finalization reserve must not underflow the provider budget"
        );
    }

    #[tokio::test]
    async fn probes_the_explicit_executable_without_path_or_auto_update() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let executable = workspace.path().join("fake-grok");
        fs::write(
            &executable,
            "#!/bin/sh\n\
             test \"$1\" = \"--version\" || exit 71\n\
             test \"$GROK_DISABLE_AUTOUPDATER\" = \"1\" || exit 72\n\
             printf 'grok 1.2.3-test [stable]\\n'\n",
        )
        .expect("probe executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("probe permissions");
        let executable = executable.canonicalize().expect("canonical executable");
        let inputs = probe_adapter_inputs(
            &BTreeMap::from([("grok".to_owned(), executable)]),
            workspace.path(),
        )
        .await
        .expect("adapter inputs");

        assert_eq!(inputs["grok"].version, "1.2.3-test");
        assert_eq!(inputs["grok"].protocol, "acp/1");
    }

    #[tokio::test]
    async fn probes_claude_version_and_subscription_auth_without_inference() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let executable = workspace.path().join("fake-claude");
        fs::write(
            &executable,
            "#!/bin/sh\n\
             test \"$DISABLE_AUTOUPDATER\" = \"1\" || exit 72\n\
             if test \"$1\" = \"--version\"; then\n\
               printf '2.1.218 (Claude Code)\\n'\n\
               exit 0\n\
             fi\n\
             test \"$1\" = \"auth\" || exit 73\n\
             test \"$2\" = \"status\" || exit 74\n\
             test \"$3\" = \"--json\" || exit 75\n\
             printf '{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"apiProvider\":\"firstParty\"}\\n'\n",
        )
        .expect("probe executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("probe permissions");
        let executable = executable.canonicalize().expect("canonical executable");
        let inputs = probe_configured_adapter_inputs(
            &BTreeMap::from([("claude".to_owned(), AdapterProbe::claude_code(executable))]),
            workspace.path(),
        )
        .await
        .expect("Claude adapter input");

        assert_eq!(inputs["claude"].version, "2.1.218");
        assert_eq!(inputs["claude"].protocol, "claude-code-stream-json/1");
    }

    #[tokio::test]
    async fn claude_probe_rejects_and_reaps_unexpected_descendants() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let executable = workspace.path().join("forking-claude");
        fs::write(
            &executable,
            "#!/bin/sh\n\
             sleep 30 </dev/null >/dev/null 2>&1 &\n\
             printf '2.1.218 (Claude Code)\\n'\n",
        )
        .expect("probe executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("probe permissions");

        let error = probe_claude_version(&executable, workspace.path())
            .await
            .expect_err("a successful probe must not leave descendants");

        assert_eq!(
            error.to_string(),
            "adapter probe spawned an unexpected child"
        );
    }

    #[tokio::test]
    async fn claude_probe_removes_inherited_billing_selectors() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let executable = workspace.path().join("environment-check");
        fs::write(
            &executable,
            "#!/bin/sh\n\
             test -z \"${ANTHROPIC_API_KEY+x}\" || exit 80\n\
             test -z \"${CLAUDE_CODE_USE_BEDROCK+x}\" || exit 81\n",
        )
        .expect("probe executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("probe permissions");
        let mut command = Command::new(executable);
        command
            .env("ANTHROPIC_API_KEY", "secret-marker")
            .env("CLAUDE_CODE_USE_BEDROCK", "1")
            .current_dir(workspace.path());
        sanitize_claude_billing_environment(&mut command);

        assert!(command.status().await.expect("environment check").success());
    }

    #[test]
    fn claude_probe_rejects_old_versions_and_non_subscription_auth() {
        assert!(normalize_claude_version(b"2.1.214 (Claude Code)\n").is_ok());
        assert!(normalize_claude_version(b"2.1.213 (Claude Code)\n").is_err());
        assert!(
            validate_claude_subscription_auth(
                br#"{"loggedIn":true,"authMethod":"apiKey","apiProvider":"firstParty"}"#
            )
            .is_err()
        );
        assert!(
            validate_claude_subscription_auth(
                br#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#
            )
            .is_ok()
        );
    }

    #[test]
    fn codex_probe_rejects_old_versions_and_non_chatgpt_auth() {
        assert!(normalize_codex_version(b"codex-cli 0.145.0\n").is_ok());
        assert!(normalize_codex_version(b"codex-cli 0.144.9\n").is_err());
        assert!(validate_codex_subscription_auth(b"Logged in using ChatGPT\n").is_ok());
        assert!(validate_codex_subscription_auth(b"Logged in using an API key\n").is_err());
        assert!(validate_codex_subscription_auth(b"Not logged in\n").is_err());
        let mut command = Command::new("true");
        sanitize_codex_billing_environment(&mut command);
    }

    #[test]
    fn executes_an_immutable_private_snapshot_of_the_locked_bytes() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let snapshot_directory = TempDir::new().expect("snapshot directory");
        let executable = workspace.path().join("fake-grok");
        fs::write(&executable, b"locked executable bytes").expect("executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("executable permissions");
        let expected_sha256 = hex::encode(Sha256::digest(b"locked executable bytes"));

        let snapshot = snapshot_locked_executable(
            AdapterProbeKind::Acp,
            "grok",
            &executable.canonicalize().expect("canonical executable"),
            &expected_sha256,
            snapshot_directory.path(),
        )
        .expect("locked snapshot");
        fs::write(&executable, b"updated executable bytes").expect("updated executable");

        assert_eq!(
            fs::read(&snapshot).expect("snapshot bytes"),
            b"locked executable bytes"
        );
        assert_eq!(
            fs::metadata(snapshot)
                .expect("snapshot metadata")
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
    }

    #[tokio::test]
    async fn refuses_provider_bootstrap_without_a_verified_repository_lock() {
        let workspace = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("workspace");
        let startup = StartupConfiguration::safe_builtins().expect("unverified startup");

        let error = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
            .await
            .err()
            .expect("unverified lock must fail");

        assert_eq!(
            error.to_string(),
            "provider startup requires a verified repository lock"
        );
    }
}
