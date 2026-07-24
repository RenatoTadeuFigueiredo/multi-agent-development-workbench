use std::{
    env, fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use rustix::{
    fs::{Mode, OFlags},
    process::getuid,
};

use crate::{merge::ConfigLayer, validate::ConfigError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePaths {
    pub user: PathBuf,
    pub repository: PathBuf,
}

impl SourcePaths {
    pub fn discover(repository_root: &Path) -> Result<Self, ConfigError> {
        require_safe_absolute(repository_root, "repository root")?;
        let user = user_configuration_path()?;
        Ok(Self {
            user,
            repository: repository_root.join(".workbench/workbench.yaml"),
        })
    }

    pub fn load_existing(&self) -> Result<Vec<ConfigLayer>, ConfigError> {
        [("user", &self.user), ("repository", &self.repository)]
            .into_iter()
            .map(|(name, path)| load_optional_layer(name, path))
            .filter_map(Result::transpose)
            .collect()
    }
}

fn user_configuration_path() -> Result<PathBuf, ConfigError> {
    #[cfg(target_os = "macos")]
    {
        let home = absolute_environment_path("HOME")?;
        Ok(home.join("Library/Application Support/Workbench/config.yaml"))
    }
    #[cfg(target_os = "linux")]
    {
        if env::var_os("XDG_CONFIG_HOME").is_some() {
            return Ok(absolute_environment_path("XDG_CONFIG_HOME")?.join("workbench/config.yaml"));
        }
        Ok(absolute_environment_path("HOME")?.join(".config/workbench/config.yaml"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(ConfigError::UnsafeSource(
            "feature 001 supports only macOS and Linux".to_owned(),
        ))
    }
}

fn absolute_environment_path(name: &str) -> Result<PathBuf, ConfigError> {
    let value =
        env::var_os(name).ok_or_else(|| ConfigError::UnsafeSource(format!("{name} is not set")))?;
    let path = PathBuf::from(value);
    require_safe_absolute(&path, name)?;
    Ok(path)
}

fn load_optional_layer(name: &str, path: &Path) -> Result<Option<ConfigLayer>, ConfigError> {
    require_safe_absolute(path, "configuration source")?;
    if !all_components_exist_without_symlinks(path)? {
        return Ok(None);
    }
    let descriptor = match rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(rustix::io::Errno::LOOP) => {
            return Err(ConfigError::UnsafeSource(format!(
                "{} must not be a symbolic link",
                path.display()
            )));
        }
        Err(error) => return Err(ConfigError::Io(error.to_string())),
    };
    let mut file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(ConfigError::UnsafeSource(format!(
            "{} must be an owned regular file without group or world write access",
            path.display()
        )));
    }
    let mut yaml = String::new();
    file.read_to_string(&mut yaml)
        .map_err(|error| ConfigError::Io(error.to_string()))?;
    ConfigLayer::from_yaml(name, &yaml).map(Some)
}

fn require_safe_absolute(path: &Path, description: &str) -> Result<(), ConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::UnsafeSource(format!(
            "{description} must be an absolute path without parent traversal"
        )));
    }
    Ok(())
}

fn all_components_exist_without_symlinks(path: &Path) -> Result<bool, ConfigError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ConfigError::UnsafeSource(format!(
                    "{} must not contain symbolic links",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(ConfigError::Io(error.to_string())),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::TempDir;

    use super::*;

    fn root(directory: &TempDir) -> PathBuf {
        directory.path().canonicalize().expect("canonical root")
    }

    fn sources(root: &Path) -> SourcePaths {
        SourcePaths {
            user: root.join("user.yaml"),
            repository: root.join("repository.yaml"),
        }
    }

    fn write_layer(path: &Path, mode: u32) {
        fs::write(path, "routing:\n  confidence_threshold: 0.9\n").expect("configuration layer");
        fs::set_permissions(path, Permissions::from_mode(mode)).expect("configuration permissions");
    }

    #[test]
    fn missing_sources_are_ignored() {
        let directory = TempDir::new().expect("temporary directory");

        let layers = sources(&root(&directory))
            .load_existing()
            .expect("missing sources");

        assert!(layers.is_empty());
    }

    #[test]
    fn owner_readable_sources_load_in_precedence_order() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let sources = sources(&root);
        write_layer(&sources.user, 0o644);
        write_layer(&sources.repository, 0o600);

        let layers = sources.load_existing().expect("configuration sources");

        assert_eq!(
            layers
                .iter()
                .map(|layer| layer.name.as_str())
                .collect::<Vec<_>>(),
            ["user", "repository"]
        );
    }

    #[test]
    fn relative_source_is_rejected_even_when_missing() {
        let sources = SourcePaths {
            user: PathBuf::from("user.yaml"),
            repository: PathBuf::from("/missing/repository.yaml"),
        };

        let error = sources
            .load_existing()
            .expect_err("relative source must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn parent_traversal_is_rejected() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let sources = SourcePaths {
            user: root.join("unused").join("..").join("user.yaml"),
            repository: root.join("repository.yaml"),
        };

        let error = sources
            .load_existing()
            .expect_err("parent traversal must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn final_symbolic_link_is_rejected_including_when_dangling() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let sources = sources(&root);
        symlink(root.join("missing-target.yaml"), &sources.user).expect("symbolic source");

        let error = sources
            .load_existing()
            .expect_err("symbolic source must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn symbolic_link_in_an_existing_parent_is_rejected() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let target = root.join("target");
        fs::create_dir(&target).expect("target directory");
        write_layer(&target.join("config.yaml"), 0o600);
        let linked = root.join("linked");
        symlink(&target, &linked).expect("symbolic parent");
        let sources = SourcePaths {
            user: linked.join("config.yaml"),
            repository: root.join("repository.yaml"),
        };

        let error = sources
            .load_existing()
            .expect_err("symbolic parent must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn non_regular_source_is_rejected() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let sources = sources(&root);
        fs::create_dir(&sources.user).expect("source directory");

        let error = sources
            .load_existing()
            .expect_err("directory source must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn group_or_world_writable_source_is_rejected() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);
        let sources = sources(&root);
        write_layer(&sources.user, 0o666);

        let error = sources
            .load_existing()
            .expect_err("writable source must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }

    #[test]
    fn discovery_rejects_repository_parent_traversal() {
        let directory = TempDir::new().expect("temporary directory");
        let root = root(&directory);

        let error = SourcePaths::discover(&root.join("unused").join(".."))
            .expect_err("unsafe repository root must fail");

        assert!(matches!(error, ConfigError::UnsafeSource(_)));
    }
}
