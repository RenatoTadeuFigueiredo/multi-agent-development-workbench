use std::env;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use rustix::{
    fs::{FlockOperation, flock},
    process::{getpid, getuid},
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const WORKSPACE_ID_BYTES: usize = 16;
const WORKSPACE_ID_DOMAIN: &[u8] = b"workbench-workspace-id-v1\0";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RuntimePaths {
    pub configuration_file: PathBuf,
    pub state_directory: PathBuf,
    pub database_file: PathBuf,
    pub endpoint_directory: PathBuf,
    pub endpoint: PathBuf,
    pub daemon_lock: PathBuf,
}

impl RuntimePaths {
    /// Resolves the platform configuration, state, and IPC paths.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository root cannot be canonicalized, the
    /// platform is unsupported, or a required environment root is missing,
    /// relative, or unsafe.
    pub fn discover(repository_root: &Path) -> Result<Self, RuntimePathError> {
        let workspace_id = workspace_id(repository_root)?;
        let home = absolute_environment_path("HOME")?;
        #[cfg(target_os = "macos")]
        {
            let state_root = home
                .join("Library")
                .join("Application Support")
                .join("Workbench")
                .join("state");
            let configuration_file = home
                .join("Library")
                .join("Application Support")
                .join("Workbench")
                .join("config.yaml");
            let temporary = absolute_environment_path("TMPDIR")?;
            return Self::from_workspace_parts(
                configuration_file,
                &state_root,
                temporary.join(format!("workbench-{}", getuid().as_raw())),
                &workspace_id,
            );
        }
        #[cfg(target_os = "linux")]
        {
            let configuration_root = optional_absolute_environment_path("XDG_CONFIG_HOME")?
                .unwrap_or_else(|| home.join(".config"));
            let state_root = optional_absolute_environment_path("XDG_STATE_HOME")?
                .unwrap_or_else(|| home.join(".local").join("state"));
            let runtime_root = absolute_environment_path("XDG_RUNTIME_DIR")?;
            return Self::from_workspace_parts(
                configuration_root.join("workbench").join("config.yaml"),
                &state_root.join("workbench"),
                runtime_root.join("workbench"),
                &workspace_id,
            );
        }
        #[allow(unreachable_code)]
        Err(RuntimePathError::UnsupportedPlatform)
    }

    /// Builds a path set from explicitly selected absolute roots.
    ///
    /// # Errors
    ///
    /// Returns an error when any supplied path is relative or traverses a
    /// parent component.
    pub fn from_parts(
        configuration_file: PathBuf,
        state_directory: PathBuf,
        endpoint_directory: PathBuf,
    ) -> Result<Self, RuntimePathError> {
        require_safe_absolute(&configuration_file)?;
        require_safe_absolute(&state_directory)?;
        require_safe_absolute(&endpoint_directory)?;
        Ok(Self {
            configuration_file,
            database_file: state_directory.join("workbench.sqlite3"),
            daemon_lock: state_directory.join("daemon.lock"),
            endpoint: endpoint_directory.join("workbench.sock"),
            state_directory,
            endpoint_directory,
        })
    }

    fn from_workspace_parts(
        configuration_file: PathBuf,
        state_root: &Path,
        endpoint_directory: PathBuf,
        workspace_id: &str,
    ) -> Result<Self, RuntimePathError> {
        let mut paths = Self::from_parts(
            configuration_file,
            state_root.join(workspace_id),
            endpoint_directory,
        )?;
        reject_unmigrated_legacy_database(state_root, &paths.database_file)?;
        paths.endpoint = paths
            .endpoint_directory
            .join(format!("{workspace_id}.sock"));
        Ok(paths)
    }

    /// Creates and verifies the owner-only state and endpoint directories.
    ///
    /// # Errors
    ///
    /// Returns an error for symbolic links, unexpected ownership, broad
    /// permissions, or filesystem failures.
    pub fn prepare(&self) -> Result<(), RuntimePathError> {
        prepare_private_directory(&self.state_directory)?;
        prepare_private_directory(&self.endpoint_directory)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SingleDaemonLock {
    file: File,
}

impl SingleDaemonLock {
    /// Acquires a kernel-released process-unique advisory lock.
    ///
    /// # Errors
    ///
    /// Returns `DaemonAlreadyRunning` when another process holds the lock, or
    /// a runtime path error when the lock file is unsafe.
    pub fn acquire(paths: &RuntimePaths) -> Result<Self, RuntimePathError> {
        require_safe_absolute(&paths.daemon_lock)?;
        let existed = match fs::symlink_metadata(&paths.daemon_lock) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(RuntimePathError::SymbolicLink(paths.daemon_lock.clone()));
                }
                if !metadata.is_file() {
                    return Err(RuntimePathError::UnsafePath(paths.daemon_lock.clone()));
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&paths.daemon_lock)
            .map_err(RuntimePathError::Io)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(RuntimePathError::UnsafePath(paths.daemon_lock.clone()));
        }
        if metadata.uid() != getuid().as_raw() {
            return Err(RuntimePathError::UnexpectedOwner(paths.daemon_lock.clone()));
        }
        if existed && metadata.permissions().mode() & 0o077 != 0 {
            return Err(RuntimePathError::BroadPermissions(
                paths.daemon_lock.clone(),
            ));
        }
        if !existed {
            file.set_permissions(Permissions::from_mode(PRIVATE_FILE_MODE))?;
        }
        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                RuntimePathError::DaemonAlreadyRunning
            } else {
                RuntimePathError::Io(io::Error::from_raw_os_error(error.raw_os_error()))
            }
        })?;
        file.set_len(0)?;
        writeln!(file, "{}", getpid().as_raw_pid())?;
        file.sync_all()?;
        Ok(Self { file })
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

#[derive(Debug, Error)]
pub enum RuntimePathError {
    #[error("required environment path is missing: {0}")]
    MissingEnvironment(&'static str),
    #[error("path is not a safe absolute path: {0}")]
    UnsafePath(PathBuf),
    #[error("path ownership does not match the current user: {0}")]
    UnexpectedOwner(PathBuf),
    #[error("path permissions are broader than owner-only: {0}")]
    BroadPermissions(PathBuf),
    #[error("a symbolic link is not allowed in a runtime path: {0}")]
    SymbolicLink(PathBuf),
    #[error("another Workbench daemon owns the runtime lock")]
    DaemonAlreadyRunning,
    #[error(
        "legacy global Workbench state requires explicit migration; export sessions with the previous release and follow the workspace-state migration runbook"
    )]
    LegacyStateRequiresMigration,
    #[error("this platform is not supported by feature 001")]
    UnsupportedPlatform,
    #[error("runtime path operation failed")]
    Io(#[from] io::Error),
}

fn optional_absolute_environment_path(
    variable: &'static str,
) -> Result<Option<PathBuf>, RuntimePathError> {
    env::var_os(variable)
        .map(PathBuf::from)
        .map(|mut path| {
            require_safe_absolute(&path)?;
            if let Ok(metadata) = fs::symlink_metadata(&path) {
                if metadata.file_type().is_symlink() {
                    return Err(RuntimePathError::SymbolicLink(path));
                }
                if metadata.uid() != getuid().as_raw() {
                    return Err(RuntimePathError::UnexpectedOwner(path));
                }
                path = fs::canonicalize(path)?;
            }
            Ok(path)
        })
        .transpose()
}

fn absolute_environment_path(variable: &'static str) -> Result<PathBuf, RuntimePathError> {
    optional_absolute_environment_path(variable)?
        .ok_or(RuntimePathError::MissingEnvironment(variable))
}

/// Derives the stable cross-client identifier for a canonical repository root.
///
/// # Errors
///
/// Returns an error when the repository root cannot be canonicalized, is not a
/// directory, or cannot be represented as UTF-8.
pub fn workspace_id(repository_root: &Path) -> Result<String, RuntimePathError> {
    let canonical_root = fs::canonicalize(repository_root)?;
    if !canonical_root.is_dir() {
        return Err(RuntimePathError::UnsafePath(canonical_root));
    }
    let canonical_utf8 = canonical_root
        .to_str()
        .ok_or_else(|| RuntimePathError::UnsafePath(canonical_root.clone()))?;
    Ok(workspace_id_for_canonical_utf8(canonical_utf8))
}

fn workspace_id_for_canonical_utf8(canonical_root: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_ID_DOMAIN);
    hasher.update(canonical_root.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..WORKSPACE_ID_BYTES])
}

fn reject_unmigrated_legacy_database(
    state_root: &Path,
    workspace_database: &Path,
) -> Result<(), RuntimePathError> {
    let legacy_database = state_root.join("workbench.sqlite3");
    let legacy_exists = match fs::symlink_metadata(legacy_database) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let workspace_exists = match fs::symlink_metadata(workspace_database) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if legacy_exists && !workspace_exists {
        return Err(RuntimePathError::LegacyStateRequiresMigration);
    }
    Ok(())
}

fn require_safe_absolute(path: &Path) -> Result<(), RuntimePathError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RuntimePathError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn prepare_private_directory(path: &Path) -> Result<(), RuntimePathError> {
    require_safe_absolute(path)?;
    verify_existing_ancestors(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => verify_private_directory(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            fs::set_permissions(path, Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
            let metadata = fs::symlink_metadata(path)?;
            verify_private_directory(path, &metadata)
        }
        Err(error) => Err(error.into()),
    }
}

fn verify_existing_ancestors(path: &Path) -> Result<(), RuntimePathError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            return Err(RuntimePathError::SymbolicLink(current));
        }
    }
    Ok(())
}

fn verify_private_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimePathError> {
    if metadata.file_type().is_symlink() {
        return Err(RuntimePathError::SymbolicLink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(RuntimePathError::UnsafePath(path.to_path_buf()));
    }
    if metadata.uid() != getuid().as_raw() {
        return Err(RuntimePathError::UnexpectedOwner(path.to_path_buf()));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RuntimePathError::BroadPermissions(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use tempfile::TempDir;

    use super::*;

    fn paths(root: &Path) -> RuntimePaths {
        let root = root.canonicalize().expect("canonical temporary root");
        RuntimePaths::from_parts(
            root.join("config").join("config.yaml"),
            root.join("state"),
            root.join("runtime"),
        )
        .expect("valid paths")
    }

    fn workspace_paths(root: &Path, repository: &Path) -> RuntimePaths {
        RuntimePaths::from_workspace_parts(
            root.join("config").join("config.yaml"),
            &root.join("state"),
            root.join("runtime"),
            &workspace_id(repository).expect("workspace ID"),
        )
        .expect("valid workspace paths")
    }

    #[test]
    fn workspace_id_matches_the_cross_client_sha256_vector() {
        assert_eq!(
            workspace_id_for_canonical_utf8("/workspace/example"),
            "daf6640544250076b29c16531feb382e"
        );
    }

    #[test]
    fn canonical_repository_aliases_share_workspace_paths() {
        let root = TempDir::new().expect("temporary root");
        let root_path = root.path().canonicalize().expect("canonical root");
        let repository = root_path.join("repository");
        fs::create_dir(&repository).expect("repository");
        let alias = root_path.join("repository-alias");
        symlink(&repository, &alias).expect("repository alias");

        let direct = workspace_paths(&root_path, &repository);
        let through_alias = workspace_paths(&root_path, &alias);

        assert_eq!(direct, through_alias);
    }

    #[test]
    fn distinct_repositories_have_isolated_runtime_paths() {
        let root = TempDir::new().expect("temporary root");
        let root_path = root.path().canonicalize().expect("canonical root");
        let first_repository = root_path.join("first-repository");
        let second_repository = root_path.join("second-repository");
        fs::create_dir(&first_repository).expect("first repository");
        fs::create_dir(&second_repository).expect("second repository");

        let first = workspace_paths(&root_path, &first_repository);
        let second = workspace_paths(&root_path, &second_repository);

        assert_eq!(first.configuration_file, second.configuration_file);
        assert_ne!(first.state_directory, second.state_directory);
        assert_ne!(first.database_file, second.database_file);
        assert_ne!(first.daemon_lock, second.daemon_lock);
        assert_ne!(first.endpoint, second.endpoint);
        assert_eq!(first.endpoint_directory, second.endpoint_directory);
    }

    #[test]
    fn legacy_global_database_requires_explicit_workspace_migration() {
        let root = TempDir::new().expect("temporary root");
        let root_path = root.path().canonicalize().expect("canonical root");
        let repository = root_path.join("repository");
        fs::create_dir(&repository).expect("repository");
        let state_root = root_path.join("state");
        fs::create_dir(&state_root).expect("state root");
        fs::write(state_root.join("workbench.sqlite3"), b"legacy").expect("legacy database");
        let workspace_id = workspace_id(&repository).expect("workspace ID");

        let error = RuntimePaths::from_workspace_parts(
            root_path.join("config").join("config.yaml"),
            &state_root,
            root_path.join("runtime"),
            &workspace_id,
        )
        .expect_err("unmigrated legacy database must fail closed");

        assert!(matches!(
            error,
            RuntimePathError::LegacyStateRequiresMigration
        ));

        let workspace_database = state_root.join(&workspace_id).join("workbench.sqlite3");
        fs::create_dir_all(
            workspace_database
                .parent()
                .expect("workspace database parent"),
        )
        .expect("workspace state");
        fs::write(workspace_database, b"migrated").expect("workspace database");
        RuntimePaths::from_workspace_parts(
            root_path.join("config").join("config.yaml"),
            &state_root,
            root_path.join("runtime"),
            &workspace_id,
        )
        .expect("existing workspace database is already explicit");
    }

    #[test]
    fn prepares_owner_only_directories() {
        let root = TempDir::new().expect("temporary root");
        let paths = paths(root.path());

        paths.prepare().expect("private directories");

        assert_eq!(
            fs::metadata(paths.state_directory)
                .expect("state metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn rejects_broad_existing_permissions() {
        let root = TempDir::new().expect("temporary root");
        let paths = paths(root.path());
        fs::create_dir(&paths.state_directory).expect("state directory");
        fs::set_permissions(&paths.state_directory, Permissions::from_mode(0o755))
            .expect("broad mode");

        let error = paths.prepare().expect_err("broad mode must fail");

        assert!(matches!(error, RuntimePathError::BroadPermissions(_)));
    }

    #[test]
    fn rejects_symbolic_link_components() {
        let root = TempDir::new().expect("temporary root");
        let root_path = root.path().canonicalize().expect("canonical root");
        let target = root_path.join("target");
        fs::create_dir(&target).expect("target directory");
        let linked = root_path.join("linked");
        symlink(&target, &linked).expect("symbolic link");
        let paths = RuntimePaths::from_parts(
            root_path.join("config.yaml"),
            linked.join("state"),
            root_path.join("runtime"),
        )
        .expect("syntactically valid paths");

        let error = paths.prepare().expect_err("symlink must fail");

        assert!(matches!(error, RuntimePathError::SymbolicLink(_)));
    }

    #[test]
    fn daemon_lock_is_exclusive_and_recoverable_after_drop() {
        let root = TempDir::new().expect("temporary root");
        let paths = paths(root.path());
        paths.prepare().expect("private directories");
        let first = SingleDaemonLock::acquire(&paths).expect("first lock");
        assert_eq!(
            fs::metadata(&paths.daemon_lock)
                .expect("daemon lock metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        assert!(matches!(
            SingleDaemonLock::acquire(&paths),
            Err(RuntimePathError::DaemonAlreadyRunning)
        ));
        drop(first);
        SingleDaemonLock::acquire(&paths).expect("lock after clean drop");
    }

    #[test]
    fn rejects_broad_existing_daemon_lock_permissions() {
        let root = TempDir::new().expect("temporary root");
        let paths = paths(root.path());
        paths.prepare().expect("private directories");
        fs::write(&paths.daemon_lock, b"stale").expect("daemon lock");
        fs::set_permissions(&paths.daemon_lock, Permissions::from_mode(0o666)).expect("broad mode");

        let error = SingleDaemonLock::acquire(&paths).expect_err("broad lock must fail");

        assert!(matches!(error, RuntimePathError::BroadPermissions(_)));
    }

    #[test]
    fn rejects_symbolic_link_daemon_lock() {
        let root = TempDir::new().expect("temporary root");
        let paths = paths(root.path());
        paths.prepare().expect("private directories");
        let target = paths.state_directory.join("lock-target");
        fs::write(&target, b"target").expect("lock target");
        symlink(&target, &paths.daemon_lock).expect("symbolic lock");

        let error = SingleDaemonLock::acquire(&paths).expect_err("symbolic lock must fail");

        assert!(matches!(error, RuntimePathError::SymbolicLink(_)));
    }
}
