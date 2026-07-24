use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    model::{ProviderType, WorkbenchConfiguration},
    snapshot::{ConfigurationSnapshot, canonical_json},
    validate::{ConfigError, validate_digest},
};

#[derive(Clone, Debug)]
pub struct AdapterInput {
    pub protocol: String,
    pub version: String,
    pub executable: PathBuf,
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
                Ok((
                    name.clone(),
                    LockedAdapter {
                        protocol: input.protocol.clone(),
                        version: input.version.clone(),
                        executable_sha256: sha256_file(&input.executable)?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        for (name, provider) in &config.providers {
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
                || adapter.executable_sha256 != sha256_file(&input.executable)?
            {
                return Err(ConfigError::Lock(format!(
                    "adapter {name} differs from its lock pin"
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

fn sha256_file(path: &Path) -> Result<String, ConfigError> {
    let bytes = fs::read(path).map_err(|error| ConfigError::Lock(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
