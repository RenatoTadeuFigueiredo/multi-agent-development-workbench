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

use rustix::process::getuid;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command};
use workbench_acp::{GrokLaunchProfile, GrokProviderAdapter};
use workbench_config::{
    ACP_PROTOCOL, AdapterInput, ConfigError, canonicalize_adapter_executable,
    model::{ApprovalMode, Capability, EffectClass, ProviderType},
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

use crate::StartupConfiguration;

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_VERSION_OUTPUT_BYTES: usize = 4_096;
const MAX_VERSION_READ_BYTES: u64 = 4_097;
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
    managed: Vec<Arc<GrokProviderAdapter>>,
    _snapshots: TempDir,
}

struct ProviderDescriptor {
    provider_id: ProviderId,
    version: String,
    protocol: String,
    executable: PathBuf,
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

        for (name, descriptor) in descriptors {
            let adapter = match GrokProviderAdapter::connect(
                descriptor.provider_id.clone(),
                descriptor.version.clone(),
                GrokLaunchProfile::new(&descriptor.executable, workspace),
                cancellation_deadline,
            )
            .await
            {
                Ok(adapter) => Arc::new(adapter),
                Err(error) => {
                    if !shutdown_managed(&managed).await {
                        return Err(ProviderRuntimeError::Reap);
                    }
                    if error.category() == workbench_core::FailureCategory::Internal {
                        return Err(ProviderRuntimeError::Reap);
                    }
                    return Err(error.into());
                }
            };
            let capabilities = match adapter.capabilities().await {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    let mut cleanup = managed.clone();
                    cleanup.push(adapter);
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
                cleanup.push(adapter);
                if !shutdown_managed(&cleanup).await {
                    return Err(ProviderRuntimeError::Reap);
                }
                return Err(ProviderRuntimeError::Incompatible(
                    "ACP adapter identity differs from the lock",
                ));
            }
            catalog.insert(name.clone(), config_capabilities(&capabilities));
            let erased: Arc<dyn ProviderAdapter> = adapter.clone();
            adapters.insert(descriptor.provider_id, erased);
            managed.push(adapter);
        }

        Ok(Self {
            registry: Arc::new(StaticProviderRegistry { adapters }),
            catalog,
            managed,
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
        if provider.kind != ProviderType::Acp {
            continue;
        }
        let provider_id = ProviderId::parse(name.clone())?;
        let executable =
            provider
                .executable
                .as_deref()
                .ok_or(ProviderRuntimeError::Incompatible(
                    "ACP executable is not configured",
                ))?;
        let locked =
            startup.base_lock.adapters.get(name).ok_or({
                ProviderRuntimeError::Incompatible("ACP adapter is absent from the lock")
            })?;
        let executable = snapshot_locked_executable(
            name,
            Path::new(executable),
            &locked.executable_sha256,
            snapshot_directory,
        )?;
        descriptors.insert(
            name.clone(),
            ProviderDescriptor {
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
    name: &str,
    source: &Path,
    expected_sha256: &str,
    snapshot_directory: &Path,
) -> Result<PathBuf, ProviderRuntimeError> {
    let (target, digest) = copy_executable_snapshot(name, source, snapshot_directory)?;
    if digest != expected_sha256 {
        return Err(ProviderRuntimeError::Incompatible(
            "ACP executable differs from the lock",
        ));
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
    let validated = executables
        .iter()
        .map(|(name, executable)| Ok((name.clone(), canonicalize_adapter_executable(executable)?)))
        .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
    let snapshots = tempfile::Builder::new()
        .prefix(".adapter-probe-")
        .tempdir_in(workspace)
        .map_err(|_| ProviderRuntimeError::Snapshot)?;
    let mut inputs = BTreeMap::new();
    for (name, executable) in validated {
        let (snapshot, executable_sha256) =
            copy_executable_snapshot(&name, &executable, snapshots.path())?;
        let version = probe_grok_version(&snapshot, workspace).await?;
        inputs.insert(
            name,
            AdapterInput {
                protocol: ACP_PROTOCOL.to_owned(),
                version,
                executable,
                executable_sha256,
            },
        );
    }
    Ok(inputs)
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

async fn shutdown_managed(adapters: &[Arc<GrokProviderAdapter>]) -> bool {
    futures_util::future::join_all(adapters.iter().rev().map(|adapter| adapter.shutdown()))
        .await
        .into_iter()
        .all(|report| report.reaped)
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

    use sha2::{Digest, Sha256};

    use super::{
        CANCELLATION_FINALIZATION_RESERVE, ProviderRuntime, normalize_grok_version,
        probe_adapter_inputs, provider_cancellation_budget, snapshot_locked_executable,
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
