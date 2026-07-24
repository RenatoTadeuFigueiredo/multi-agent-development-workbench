use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use rustix::process::getuid;
use serde_json::Value;
use workbench_config::{
    AdapterInput, ConfigError, ConfigLayer, ConfigurationSnapshot, ResolvedConfiguration,
    WorkbenchConfiguration, WorkbenchLock, canonicalize_adapter_executable,
    merge::{resolve, resolve_with_builtins},
    model::ProviderType,
    source::SourcePaths,
};

#[derive(Clone, Debug)]
pub struct StartupConfiguration {
    pub resolved: WorkbenchConfiguration,
    pub snapshot: ConfigurationSnapshot,
    pub base_lock: WorkbenchLock,
    pub sources: Vec<String>,
    pub(crate) lock_verified: bool,
}

impl StartupConfiguration {
    pub(crate) const fn lock_is_verified(&self) -> bool {
        self.lock_verified
    }

    /// Resolves the canonical executables that require an explicit adapter
    /// probe before a repository lock can be generated.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, missing ACP
    /// executables, or executables that fail canonical safety validation.
    pub fn adapter_executables(
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
    ) -> Result<BTreeMap<String, PathBuf>, ConfigError> {
        let resolved = resolve_startup_configuration(repository_root, explicit_configuration)?;
        resolved
            .configuration
            .providers
            .iter()
            .filter(|(_, provider)| provider.kind == ProviderType::Acp)
            .map(|(name, provider)| {
                let executable = provider.executable.as_deref().ok_or_else(|| {
                    ConfigError::Lock(format!("ACP provider {name} has no executable"))
                })?;
                Ok((
                    name.clone(),
                    canonicalize_adapter_executable(Path::new(executable))?,
                ))
            })
            .collect()
    }

    /// Resolves repository configuration and verifies its committed base lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, invalid layers, or a
    /// missing or mismatched repository lock.
    pub fn load(repository_root: &Path) -> Result<Self, ConfigError> {
        Self::load_with_configuration(repository_root, None)
    }

    /// Resolves all configuration layers, including an optional explicit
    /// layer, and verifies the repository lock against that exact result.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, invalid layers, or a
    /// missing or mismatched repository lock.
    pub fn load_with_configuration(
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        let resolved = resolve_startup_configuration(repository_root, explicit_configuration)?;
        let committed = read_repository_lock(repository_root)?;
        let adapter_inputs = adapter_inputs_from_lock(&resolved.configuration, &committed)?;
        let expected = startup_from_resolved(resolved, &adapter_inputs)?;
        verify_expected_lock(expected, committed)
    }

    /// Resolves all configuration layers and verifies their adapter identities
    /// against an existing repository lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, invalid adapter
    /// inputs, or a missing or mismatched repository lock.
    pub fn load_with_configuration_and_adapter_inputs(
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
        adapter_inputs: &BTreeMap<String, AdapterInput>,
    ) -> Result<Self, ConfigError> {
        let expected = Self::inspect_with_adapter_inputs(
            repository_root,
            explicit_configuration,
            adapter_inputs,
        )?;
        verify_expected_lock(expected, read_repository_lock(repository_root)?)
    }

    /// Resolves configuration without requiring an existing repository lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, invalid layers, or
    /// a lock model that cannot represent the resolved configuration.
    pub fn inspect(
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        Self::inspect_with_adapter_inputs(repository_root, explicit_configuration, &BTreeMap::new())
    }

    /// Resolves configuration and creates a deterministic lock from explicit
    /// adapter probe inputs without requiring an existing repository lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for unsafe sources, invalid layers,
    /// unsafe executables, or adapter inputs that do not match providers.
    pub fn inspect_with_adapter_inputs(
        repository_root: &Path,
        explicit_configuration: Option<&Path>,
        adapter_inputs: &BTreeMap<String, AdapterInput>,
    ) -> Result<Self, ConfigError> {
        startup_from_resolved(
            resolve_startup_configuration(repository_root, explicit_configuration)?,
            adapter_inputs,
        )
    }

    /// Atomically writes this resolved repository lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the target directory or lock path
    /// is unsafe, or when the durable atomic write fails.
    pub fn write_base_lock(&self, repository_root: &Path) -> Result<(), ConfigError> {
        require_absolute_repository(repository_root)?;
        let directory = repository_root.join(".workbench");
        if let Ok(metadata) = fs::symlink_metadata(&directory)
            && (metadata.file_type().is_symlink() || !metadata.is_dir())
        {
            return Err(ConfigError::UnsafeSource(
                ".workbench must be a non-symlink directory".to_owned(),
            ));
        }
        fs::create_dir_all(&directory).map_err(|error| ConfigError::Io(error.to_string()))?;
        let parent = open_owned_directory(&directory)?;
        let lock_path = directory.join("workbench.lock");
        match fs::symlink_metadata(&lock_path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.uid() != getuid().as_raw() =>
            {
                return Err(ConfigError::UnsafeSource(
                    "workbench.lock must be an owned regular non-symlink file".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::Io(error.to_string())),
        }
        let temporary = directory.join(format!(".workbench.lock.{}.tmp", uuid::Uuid::now_v7()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)
                .map_err(|error| ConfigError::Io(error.to_string()))?;
            let canonical = workbench_config::snapshot::canonical_json(&self.base_lock)?;
            file.write_all(canonical.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.sync_all())
                .map_err(|error| ConfigError::Io(error.to_string()))?;
            fs::rename(&temporary, &lock_path)
                .map_err(|error| ConfigError::Io(error.to_string()))?;
            let metadata = fs::symlink_metadata(&lock_path)
                .map_err(|error| ConfigError::Io(error.to_string()))?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.uid() != getuid().as_raw()
                || metadata.permissions().mode() & 0o777 != 0o600
            {
                return Err(ConfigError::UnsafeSource(
                    "written workbench.lock is unsafe".to_owned(),
                ));
            }
            parent
                .sync_all()
                .map_err(|error| ConfigError::Io(error.to_string()))
        })();
        if result.is_err() {
            let _ignored = fs::remove_file(&temporary);
        }
        result
    }

    /// Resolves the deterministic offline fake-provider profile.
    ///
    /// # Errors
    ///
    /// Returns a configuration error if the embedded profile is invalid.
    pub fn safe_builtins() -> Result<Self, ConfigError> {
        let resolved = resolve_with_builtins(&[])?;
        let snapshot =
            ConfigurationSnapshot::create(&resolved.configuration, resolved.sources.clone())?;
        let base_lock =
            WorkbenchLock::repository(&resolved.configuration, &snapshot, &BTreeMap::new())?;
        Ok(Self {
            resolved: resolved.configuration,
            snapshot,
            base_lock,
            sources: resolved.sources,
            lock_verified: false,
        })
    }

    /// Applies validated session overrides and creates the linked lock.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when an override, snapshot, or lock
    /// violates the committed schema and base-lock restrictions.
    pub fn resolve_session(
        &self,
        overrides: Option<&std::collections::HashMap<String, Value>>,
    ) -> Result<(ResolvedConfiguration, ConfigurationSnapshot, WorkbenchLock), ConfigError> {
        let base = ConfigLayer::from_configuration("resolved-base", &self.resolved)?;
        let resolved = if let Some(overrides) = overrides {
            let yaml = serde_yaml_ng::to_string(overrides)
                .map_err(|error| ConfigError::Syntax(error.to_string()))?;
            let override_layer = ConfigLayer::from_yaml("session", &yaml)?;
            resolve(&[base, override_layer])?
        } else {
            resolve(&[base])?
        };
        reject_adapter_executable_overrides(&self.resolved, &resolved.configuration)?;
        self.base_lock
            .verify_configured_executables(&resolved.configuration)?;
        let mut sources = self.sources.clone();
        if overrides.is_some() {
            sources.push("session".to_owned());
        }
        let snapshot = ConfigurationSnapshot::create(&resolved.configuration, sources)?;
        let session_lock =
            WorkbenchLock::session(&self.base_lock, &resolved.configuration, &snapshot)?;
        session_lock.verify_linked_to(&self.base_lock)?;
        Ok((resolved, snapshot, session_lock))
    }
}

fn resolve_startup_configuration(
    repository_root: &Path,
    explicit_configuration: Option<&Path>,
) -> Result<ResolvedConfiguration, ConfigError> {
    require_absolute_repository(repository_root)?;
    let mut layers = SourcePaths::discover(repository_root)?.load_existing()?;
    if let Some(path) = explicit_configuration {
        layers.push(load_explicit_layer(path)?);
    }
    resolve_with_builtins(&layers)
}

fn startup_from_resolved(
    resolved: ResolvedConfiguration,
    adapter_inputs: &BTreeMap<String, AdapterInput>,
) -> Result<StartupConfiguration, ConfigError> {
    let snapshot =
        ConfigurationSnapshot::create(&resolved.configuration, resolved.sources.clone())?;
    let base_lock = WorkbenchLock::repository(&resolved.configuration, &snapshot, adapter_inputs)?;
    Ok(StartupConfiguration {
        resolved: resolved.configuration,
        snapshot,
        base_lock,
        sources: resolved.sources,
        lock_verified: false,
    })
}

fn read_repository_lock(repository_root: &Path) -> Result<WorkbenchLock, ConfigError> {
    let lock_path = repository_root.join(".workbench/workbench.lock");
    let bytes = read_owned_regular_file(&lock_path, true).map_err(|_| {
        ConfigError::Lock("repository lock is missing, unsafe, or unreadable".to_owned())
    })?;
    let committed: WorkbenchLock = serde_json::from_slice(&bytes)
        .map_err(|_| ConfigError::Lock("repository lock is invalid".to_owned()))?;
    committed.verify()?;
    Ok(committed)
}

fn adapter_inputs_from_lock(
    configuration: &WorkbenchConfiguration,
    committed: &WorkbenchLock,
) -> Result<BTreeMap<String, AdapterInput>, ConfigError> {
    committed
        .adapters
        .iter()
        .map(|(name, locked)| {
            let provider = configuration.providers.get(name).ok_or_else(|| {
                ConfigError::Lock(format!("locked adapter {name} has no configured provider"))
            })?;
            let executable = provider.executable.as_deref().ok_or_else(|| {
                ConfigError::Lock(format!(
                    "locked adapter {name} has no configured executable"
                ))
            })?;
            Ok((
                name.clone(),
                AdapterInput {
                    protocol: locked.protocol.clone(),
                    version: locked.version.clone(),
                    executable: PathBuf::from(executable),
                    executable_sha256: locked.executable_sha256.clone(),
                },
            ))
        })
        .collect()
}

fn verify_expected_lock(
    expected: StartupConfiguration,
    committed: WorkbenchLock,
) -> Result<StartupConfiguration, ConfigError> {
    if committed != expected.base_lock {
        return Err(ConfigError::Lock(
            "repository lock differs from resolved configuration".to_owned(),
        ));
    }
    Ok(StartupConfiguration {
        base_lock: committed,
        lock_verified: true,
        ..expected
    })
}

fn reject_adapter_executable_overrides(
    base: &WorkbenchConfiguration,
    session: &WorkbenchConfiguration,
) -> Result<(), ConfigError> {
    for (name, provider) in &session.providers {
        if provider.kind == ProviderType::Acp
            && base
                .providers
                .get(name)
                .is_none_or(|provider| provider.kind != ProviderType::Acp)
        {
            return Err(ConfigError::Lock(format!(
                "session override introduced ACP provider {name}"
            )));
        }
    }
    for (name, provider) in &base.providers {
        if provider.kind != ProviderType::Acp {
            continue;
        }
        let candidate = session.providers.get(name).ok_or_else(|| {
            ConfigError::Lock(format!(
                "session override removed pinned ACP provider {name}"
            ))
        })?;
        if candidate.kind != ProviderType::Acp {
            return Err(ConfigError::Lock(format!(
                "session override changed pinned ACP provider {name}"
            )));
        }
        let base_executable = provider
            .executable
            .as_deref()
            .ok_or_else(|| ConfigError::Lock(format!("ACP provider {name} has no executable")))?;
        let candidate_executable = candidate
            .executable
            .as_deref()
            .ok_or_else(|| ConfigError::Lock(format!("ACP provider {name} has no executable")))?;
        if canonicalize_adapter_executable(Path::new(base_executable))?
            != canonicalize_adapter_executable(Path::new(candidate_executable))?
        {
            return Err(ConfigError::Lock(format!(
                "session override replaced pinned ACP executable for provider {name}"
            )));
        }
    }
    Ok(())
}

fn require_absolute_repository(repository_root: &Path) -> Result<(), ConfigError> {
    if !repository_root.is_absolute()
        || repository_root
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::UnsafeSource(
            "repository root must be absolute".to_owned(),
        ));
    }
    verify_no_symlink_components(repository_root)
}

fn load_explicit_layer(path: &Path) -> Result<ConfigLayer, ConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::UnsafeSource(
            "explicit configuration path must be absolute without parent traversal".to_owned(),
        ));
    }
    let bytes = read_owned_regular_file(path, true)?;
    let yaml = String::from_utf8(bytes)
        .map_err(|_| ConfigError::Syntax("explicit configuration is not UTF-8".to_owned()))?;
    ConfigLayer::from_yaml("explicit", &yaml)
}

fn read_owned_regular_file(
    path: &Path,
    reject_group_or_world_writable: bool,
) -> Result<Vec<u8>, ConfigError> {
    verify_no_symlink_components(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || (reject_group_or_world_writable && metadata.permissions().mode() & 0o022 != 0)
    {
        return Err(ConfigError::UnsafeSource(
            "configuration source ownership, type, or permissions are unsafe".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    Ok(bytes)
}

fn verify_no_symlink_components(path: &Path) -> Result<(), ConfigError> {
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ConfigError::UnsafeSource(
                    "configuration paths must not contain symbolic links".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::Io(error.to_string())),
        }
    }
    Ok(())
}

fn open_owned_directory(path: &Path) -> Result<fs::File, ConfigError> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    let metadata = directory
        .metadata()
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    if !metadata.is_dir()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(ConfigError::UnsafeSource(
            ".workbench must be an owned non-symlink directory without group or world write access"
                .to_owned(),
        ));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
    };

    use tempfile::TempDir;
    use workbench_config::model::Provider;

    use super::*;

    fn repository_root(repository: &TempDir) -> PathBuf {
        repository
            .path()
            .canonicalize()
            .expect("canonical repository")
    }

    fn write_explicit_configuration(path: &Path, mode: u32) {
        let mut configuration = StartupConfiguration::safe_builtins()
            .expect("builtins")
            .resolved;
        configuration.routing.confidence_threshold = 0.91;
        write_configuration(path, &configuration, mode);
    }

    fn write_configuration(path: &Path, configuration: &WorkbenchConfiguration, mode: u32) {
        fs::write(
            path,
            serde_yaml_ng::to_string(configuration).expect("configuration YAML"),
        )
        .expect("explicit configuration");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("configuration mode");
    }

    fn acp_configuration(executable: &Path) -> WorkbenchConfiguration {
        let mut configuration = StartupConfiguration::safe_builtins()
            .expect("builtins")
            .resolved;
        configuration.providers.insert(
            "grok".to_owned(),
            Provider {
                kind: ProviderType::Acp,
                executable: Some(executable.to_string_lossy().into_owned()),
                credential_ref: None,
                privacy: None,
            },
        );
        configuration
    }

    #[test]
    fn requires_the_repository_lock() {
        let repository = TempDir::new().expect("repository");
        let error = StartupConfiguration::load(&repository_root(&repository))
            .expect_err("lock is required");
        assert!(matches!(error, ConfigError::Lock(_)));
    }

    #[test]
    fn loads_a_matching_repository_lock() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let builtins = StartupConfiguration::safe_builtins().expect("builtins");
        fs::create_dir(root.join(".workbench")).expect("workbench directory");
        fs::write(
            root.join(".workbench/workbench.lock"),
            serde_json::to_vec(&builtins.base_lock).expect("lock JSON"),
        )
        .expect("write lock");
        StartupConfiguration::load(&root).expect("matching lock");
    }

    #[test]
    fn writes_an_atomic_owner_only_repository_lock() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let inspected = StartupConfiguration::inspect(&root, None).expect("configuration");

        inspected.write_base_lock(&root).expect("repository lock");

        let lock_path = root.join(".workbench/workbench.lock");
        let metadata = fs::symlink_metadata(&lock_path).expect("lock metadata");
        assert!(metadata.is_file());
        assert!(!metadata.file_type().is_symlink());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(
            fs::read_to_string(&lock_path)
                .expect("lock contents")
                .ends_with('\n')
        );
        assert!(
            fs::read_dir(root.join(".workbench"))
                .expect("workbench directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
        StartupConfiguration::load(&root).expect("written lock loads");
    }

    #[test]
    fn rejects_relative_explicit_configuration_path() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);

        let error = StartupConfiguration::inspect(&root, Some(Path::new("config.yaml")))
            .expect_err("relative configuration must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn rejects_parent_traversal_in_explicit_configuration_path() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let explicit = root.join("unused").join("..").join("configuration.yaml");

        let error = StartupConfiguration::inspect(&root, Some(&explicit))
            .expect_err("parent traversal must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn rejects_symbolic_link_explicit_configuration() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let target = root.join("configuration-target.yaml");
        write_explicit_configuration(&target, 0o600);
        let linked = root.join("configuration.yaml");
        symlink(&target, &linked).expect("configuration symbolic link");

        let error = StartupConfiguration::inspect(&root, Some(&linked))
            .expect_err("symbolic configuration must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn rejects_group_or_world_writable_explicit_configuration() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let explicit = root.join("configuration.yaml");
        write_explicit_configuration(&explicit, 0o666);

        let error = StartupConfiguration::inspect(&root, Some(&explicit))
            .expect_err("writable configuration must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn verifies_repository_lock_against_the_explicit_layer() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        let explicit = root.join("configuration.yaml");
        write_explicit_configuration(&explicit, 0o600);
        let inspected =
            StartupConfiguration::inspect(&root, Some(&explicit)).expect("configuration");
        inspected.write_base_lock(&root).expect("repository lock");

        let loaded = StartupConfiguration::load_with_configuration(&root, Some(&explicit))
            .expect("matching explicit lock");

        assert!((loaded.resolved.routing.confidence_threshold - 0.91).abs() < f64::EPSILON);
        assert!(matches!(
            StartupConfiguration::load(&root),
            Err(ConfigError::Lock(_))
        ));
    }

    #[test]
    fn refuses_to_replace_a_symbolic_link_repository_lock() {
        let repository = TempDir::new().expect("repository");
        let root = repository_root(&repository);
        fs::create_dir(root.join(".workbench")).expect("workbench directory");
        symlink(
            root.join("missing-lock-target"),
            root.join(".workbench/workbench.lock"),
        )
        .expect("symbolic lock");
        let inspected = StartupConfiguration::inspect(&root, None).expect("configuration");

        let error = inspected
            .write_base_lock(&root)
            .expect_err("symbolic lock must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn session_overrides_link_to_the_base_lock() {
        let startup = StartupConfiguration::safe_builtins().expect("builtins");
        let overrides = HashMap::from([(
            "routing".to_owned(),
            serde_json::json!({"confidence_threshold": 0.9}),
        )]);
        let (_, _, session_lock) = startup
            .resolve_session(Some(&overrides))
            .expect("session config");
        session_lock
            .verify_linked_to(&startup.base_lock)
            .expect("linked lock");
    }

    #[test]
    fn acp_startup_pins_and_rechecks_the_explicit_executable() {
        let repository = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("repository");
        let root = repository_root(&repository);
        let executable = root.join("fake-acp");
        let spawn_marker = root.join("fake-acp-was-spawned");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf spawned > \"{}\"\n",
                spawn_marker.display()
            ),
        )
        .expect("fake executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("executable permissions");
        let configuration_path = root.join("configuration.yaml");
        write_configuration(&configuration_path, &acp_configuration(&executable), 0o600);
        assert_eq!(
            StartupConfiguration::adapter_executables(&root, Some(&configuration_path))
                .expect("resolved adapter executables"),
            BTreeMap::from([(
                "grok".to_owned(),
                executable.canonicalize().expect("canonical executable")
            )])
        );
        let adapter_inputs = BTreeMap::from([(
            "grok".to_owned(),
            AdapterInput::acp(&executable, "0.2.7").expect("adapter input"),
        )]);
        let inspected = StartupConfiguration::inspect_with_adapter_inputs(
            &root,
            Some(&configuration_path),
            &adapter_inputs,
        )
        .expect("inspected ACP configuration");
        inspected.write_base_lock(&root).expect("repository lock");

        StartupConfiguration::load_with_configuration_and_adapter_inputs(
            &root,
            Some(&configuration_path),
            &adapter_inputs,
        )
        .expect("matching ACP lock");
        StartupConfiguration::load_with_configuration(&root, Some(&configuration_path))
            .expect("matching ACP lock using pinned adapter identity");
        assert!(
            !spawn_marker.exists(),
            "static lock verification must not spawn the ACP executable"
        );

        fs::write(
            &executable,
            format!(
                "#!/bin/sh\n# replacement\nprintf spawned > \"{}\"\n",
                spawn_marker.display()
            ),
        )
        .expect("replace executable");
        assert!(matches!(
            StartupConfiguration::load_with_configuration(&root, Some(&configuration_path)),
            Err(ConfigError::Lock(_))
        ));
        assert!(matches!(
            StartupConfiguration::load_with_configuration_and_adapter_inputs(
                &root,
                Some(&configuration_path),
                &adapter_inputs,
            ),
            Err(ConfigError::Lock(_))
        ));
        assert!(
            !spawn_marker.exists(),
            "digest rejection must happen without spawning the ACP executable"
        );
    }

    #[test]
    fn session_override_cannot_replace_a_pinned_acp_executable() {
        let repository = TempDir::new_in(std::env::current_dir().expect("current directory"))
            .expect("repository");
        let root = repository_root(&repository);
        let executable = root.join("fake-acp");
        let replacement = root.join("replacement-acp");
        fs::write(&executable, b"same executable bytes").expect("fake executable");
        fs::write(&replacement, b"same executable bytes").expect("replacement executable");
        for path in [&executable, &replacement] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .expect("executable permissions");
        }
        let configuration_path = root.join("configuration.yaml");
        write_configuration(&configuration_path, &acp_configuration(&executable), 0o600);
        let adapter_inputs = BTreeMap::from([(
            "grok".to_owned(),
            AdapterInput::acp(&executable, "0.2.7").expect("adapter input"),
        )]);
        let startup = StartupConfiguration::inspect_with_adapter_inputs(
            &root,
            Some(&configuration_path),
            &adapter_inputs,
        )
        .expect("inspected ACP configuration");
        let overrides = HashMap::from([(
            "providers".to_owned(),
            serde_json::json!({"grok": {"executable": replacement}}),
        )]);

        let error = startup
            .resolve_session(Some(&overrides))
            .expect_err("replacement must fail");

        assert!(matches!(error, ConfigError::Lock(_)));
    }
}
