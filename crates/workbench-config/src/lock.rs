use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Component,
    path::{Path, PathBuf},
};

use rustix::{
    fs::{Mode, OFlags},
    process::getuid,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    model::{ProviderType, WorkbenchConfiguration},
    snapshot::{ConfigurationSnapshot, canonical_json},
    validate::{ConfigError, validate_digest},
};

pub const ACP_PROTOCOL: &str = "acp/1";

#[derive(Clone, Debug)]
pub struct AdapterInput {
    pub protocol: String,
    pub version: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
}

impl AdapterInput {
    /// Creates an ACP version 1 input after resolving a safe executable.
    pub fn acp(
        executable: impl AsRef<Path>,
        version: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        let executable = canonicalize_adapter_executable(executable.as_ref())?;
        Ok(Self {
            protocol: ACP_PROTOCOL.to_owned(),
            version: version.into(),
            executable_sha256: sha256_file(&executable)?,
            executable,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchLock {
    pub version: u32,
    pub scope: LockScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_lock_hash: Option<String>,
    pub configuration: LockedConfiguration,
    pub protocol: LockedProtocol,
    pub adapters: BTreeMap<String, LockedAdapter>,
    pub models: BTreeMap<String, LockedModel>,
    pub mcps: BTreeMap<String, LockedMcp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockScope {
    Repository,
    Session,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedConfiguration {
    pub schema_version: u32,
    pub hash_algorithm: String,
    pub resolved_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedProtocol {
    pub major: u32,
    pub minor: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedAdapter {
    pub protocol: String,
    pub version: String,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedModel {
    pub provider: String,
    pub runtime_model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedMcp {
    pub version: String,
    pub sha256: String,
}

impl WorkbenchLock {
    pub fn repository(
        config: &WorkbenchConfiguration,
        snapshot: &ConfigurationSnapshot,
        adapter_inputs: &BTreeMap<String, AdapterInput>,
    ) -> Result<Self, ConfigError> {
        let adapters = adapter_inputs
            .iter()
            .map(|(name, input)| {
                let provider = config.providers.get(name).ok_or_else(|| {
                    ConfigError::Lock(format!(
                        "adapter input {name} has no matching configured provider"
                    ))
                })?;
                let configured = provider.executable.as_deref().ok_or_else(|| {
                    ConfigError::Lock(format!("adapter input {name} has no configured executable"))
                })?;
                let configured = canonicalize_adapter_executable(Path::new(configured))?;
                let executable = canonicalize_adapter_executable(&input.executable)?;
                if configured != executable {
                    return Err(ConfigError::Lock(format!(
                        "adapter input {name} differs from its configured executable"
                    )));
                }
                validate_adapter_identity(name, provider.kind, input)?;
                validate_digest(
                    &input.executable_sha256,
                    &format!("adapter_inputs.{name}.executable_sha256"),
                )?;
                if sha256_file(&executable)? != input.executable_sha256 {
                    return Err(ConfigError::Lock(format!(
                        "adapter input {name} changed after identity capture"
                    )));
                }
                Ok((
                    name.clone(),
                    LockedAdapter {
                        protocol: input.protocol.clone(),
                        version: input.version.clone(),
                        executable_sha256: input.executable_sha256.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        for (name, provider) in &config.providers {
            if provider.kind == ProviderType::Acp && provider.executable.is_none() {
                return Err(ConfigError::Lock(format!(
                    "ACP provider {name} has no configured executable"
                )));
            }
            if provider.kind != ProviderType::Fake
                && provider.executable.is_some()
                && !adapters.contains_key(name)
            {
                return Err(ConfigError::Lock(format!(
                    "provider {name} has no pinned adapter executable"
                )));
            }
        }
        Ok(Self {
            version: 1,
            scope: LockScope::Repository,
            base_lock_hash: None,
            configuration: locked_configuration(snapshot),
            protocol: LockedProtocol { major: 1, minor: 0 },
            adapters,
            models: lock_models(config),
            mcps: lock_mcps(config),
        })
    }

    pub fn session(
        base: &Self,
        config: &WorkbenchConfiguration,
        snapshot: &ConfigurationSnapshot,
    ) -> Result<Self, ConfigError> {
        for (name, provider) in &config.providers {
            if provider.kind != ProviderType::Fake
                && provider.executable.is_some()
                && !base.adapters.contains_key(name)
            {
                return Err(ConfigError::Lock(format!(
                    "session override introduced unpinned adapter {name}"
                )));
            }
        }
        if config
            .mcp_servers
            .keys()
            .any(|name| !base.mcps.contains_key(name))
        {
            return Err(ConfigError::Lock(
                "session override introduced an MCP absent from the base lock".to_owned(),
            ));
        }
        Ok(Self {
            version: 1,
            scope: LockScope::Session,
            base_lock_hash: Some(base.hash()?),
            configuration: locked_configuration(snapshot),
            protocol: base.protocol.clone(),
            adapters: base.adapters.clone(),
            models: lock_models(config),
            mcps: lock_mcps(config),
        })
    }

    pub fn hash(&self) -> Result<String, ConfigError> {
        let canonical = canonical_json(self)?;
        Ok(blake3::hash(canonical.as_bytes()).to_hex().to_string())
    }

    pub fn verify(&self) -> Result<(), ConfigError> {
        if self.version != 1 || self.configuration.schema_version != 1 {
            return Err(ConfigError::Lock("unsupported lock version".to_owned()));
        }
        if self.configuration.hash_algorithm != "blake3-256" {
            return Err(ConfigError::Lock(
                "unsupported lock hash algorithm".to_owned(),
            ));
        }
        if self.protocol.major != 1 {
            return Err(ConfigError::Lock("unsupported protocol major".to_owned()));
        }
        validate_digest(
            &self.configuration.resolved_hash,
            "lock.configuration.resolved_hash",
        )?;
        for (name, adapter) in &self.adapters {
            if adapter.protocol.is_empty()
                || adapter.version.is_empty()
                || adapter.version.len() > 255
                || adapter.version.chars().any(char::is_control)
            {
                return Err(ConfigError::Lock(format!(
                    "locked adapter {name} has an invalid identity"
                )));
            }
            validate_digest(
                &adapter.executable_sha256,
                &format!("lock.adapters.{name}.executable_sha256"),
            )?;
        }
        for (name, mcp) in &self.mcps {
            validate_digest(&mcp.sha256, &format!("lock.mcps.{name}.sha256"))?;
        }
        if let Some(base_hash) = &self.base_lock_hash {
            validate_digest(base_hash, "lock.base_lock_hash")?;
        }
        match self.scope {
            LockScope::Repository if self.base_lock_hash.is_some() => Err(ConfigError::Lock(
                "repository lock cannot have a base hash".to_owned(),
            )),
            LockScope::Session if self.base_lock_hash.is_none() => Err(ConfigError::Lock(
                "session lock requires a base hash".to_owned(),
            )),
            _ => Ok(()),
        }
    }

    pub fn verify_executables(
        &self,
        adapter_inputs: &BTreeMap<String, AdapterInput>,
    ) -> Result<(), ConfigError> {
        self.verify()?;
        if self.adapters.len() != adapter_inputs.len() {
            return Err(ConfigError::Lock(
                "adapter inputs do not match the lock".to_owned(),
            ));
        }
        for (name, adapter) in &self.adapters {
            let input = adapter_inputs.get(name).ok_or_else(|| {
                ConfigError::Lock(format!("locked adapter {name} has no executable input"))
            })?;
            if adapter.protocol != input.protocol
                || adapter.version != input.version
                || adapter.executable_sha256 != input.executable_sha256
                || input.executable_sha256
                    != sha256_file(&canonicalize_adapter_executable(&input.executable)?)?
            {
                return Err(ConfigError::Lock(format!(
                    "adapter {name} differs from its lock pin"
                )));
            }
        }
        Ok(())
    }

    pub fn verify_configured_executables(
        &self,
        config: &WorkbenchConfiguration,
    ) -> Result<(), ConfigError> {
        self.verify()?;
        for (name, provider) in &config.providers {
            if provider.kind != ProviderType::Acp {
                continue;
            }
            let executable = provider.executable.as_deref().ok_or_else(|| {
                ConfigError::Lock(format!("ACP provider {name} has no executable"))
            })?;
            let adapter = self.adapters.get(name).ok_or_else(|| {
                ConfigError::Lock(format!("ACP provider {name} has no pinned adapter"))
            })?;
            let executable = canonicalize_adapter_executable(Path::new(executable))?;
            if adapter.protocol != ACP_PROTOCOL
                || adapter.executable_sha256 != sha256_file(&executable)?
            {
                return Err(ConfigError::Lock(format!(
                    "ACP provider {name} differs from its lock pin"
                )));
            }
        }
        Ok(())
    }

    pub fn verify_linked_to(&self, base: &Self) -> Result<(), ConfigError> {
        self.verify()?;
        base.verify()?;
        if self.scope != LockScope::Session || self.base_lock_hash.as_deref() != Some(&base.hash()?)
        {
            return Err(ConfigError::Lock(
                "session lock does not link to the supplied base lock".to_owned(),
            ));
        }
        Ok(())
    }
}

fn locked_configuration(snapshot: &ConfigurationSnapshot) -> LockedConfiguration {
    LockedConfiguration {
        schema_version: 1,
        hash_algorithm: "blake3-256".to_owned(),
        resolved_hash: snapshot.content_hash.clone(),
    }
}

fn lock_models(config: &WorkbenchConfiguration) -> BTreeMap<String, LockedModel> {
    config
        .models
        .iter()
        .map(|(name, model)| {
            (
                name.clone(),
                LockedModel {
                    provider: model.provider.clone(),
                    runtime_model: model.runtime_model.clone(),
                },
            )
        })
        .collect()
}

fn lock_mcps(config: &WorkbenchConfiguration) -> BTreeMap<String, LockedMcp> {
    config
        .mcp_servers
        .iter()
        .map(|(name, mcp)| {
            (
                name.clone(),
                LockedMcp {
                    version: mcp.version.clone(),
                    sha256: mcp.sha256.clone(),
                },
            )
        })
        .collect()
}

fn validate_adapter_identity(
    name: &str,
    provider_type: ProviderType,
    input: &AdapterInput,
) -> Result<(), ConfigError> {
    if input.protocol.is_empty()
        || input.version.is_empty()
        || input.version.len() > 255
        || input.version.chars().any(char::is_control)
    {
        return Err(ConfigError::Lock(format!(
            "adapter {name} reported an invalid version"
        )));
    }
    if provider_type == ProviderType::Acp && input.protocol != ACP_PROTOCOL {
        return Err(ConfigError::Lock(format!(
            "ACP adapter {name} must use protocol {ACP_PROTOCOL}"
        )));
    }
    Ok(())
}

pub fn canonicalize_adapter_executable(path: &Path) -> Result<PathBuf, ConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::UnsafeSource(
            "adapter executable must be an absolute path without parent traversal".to_owned(),
        ));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(ConfigError::UnsafeSource(format!(
                "adapter executable {} must not contain symbolic links",
                path.display()
            )));
        }
        if metadata.is_dir() && metadata.permissions().mode() & 0o022 != 0 {
            return Err(ConfigError::UnsafeSource(format!(
                "adapter executable {} must not traverse writable directories",
                path.display()
            )));
        }
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    let descriptor = rustix::fs::open(
        &canonical,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    let file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(ConfigError::UnsafeSource(format!(
            "adapter executable {} must be an executable regular file without group or world write access",
            path.display()
        )));
    }
    Ok(canonical)
}

fn sha256_file(path: &Path) -> Result<String, ConfigError> {
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ConfigError::Lock(error.to_string()))?;
    let mut file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::Lock(error.to_string()))?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(ConfigError::UnsafeSource(format!(
            "adapter executable {} became unsafe",
            path.display()
        )));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ConfigError::Lock(error.to_string()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}
