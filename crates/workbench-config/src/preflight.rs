use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    model::{ApprovalMode, Capability, EffectClass, WorkbenchConfiguration},
    validate::{ConfigError, validate_identifier},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub adapter_id: String,
    pub adapter_version: String,
    pub protocol: String,
    pub authentication: Authentication,
    pub capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
    pub operations: Vec<ProviderOperation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Authentication {
    Available,
    Unavailable,
    Expired,
    InteractiveRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOperation {
    pub name: String,
    pub effect_class: EffectClass,
    pub idempotent: bool,
    pub material_cost: bool,
    pub approval: ApprovalMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    pub role: String,
    pub model_alias: String,
    pub provider: String,
    pub runtime_model: String,
    pub used_fallback: bool,
}

impl ProviderCapabilities {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.adapter_id.is_empty()
            || self.adapter_version.is_empty()
            || self.protocol.is_empty()
            || self.operations.is_empty()
        {
            return Err(ConfigError::Invalid {
                path: "provider_capabilities".to_owned(),
                message: "adapter fields and operations must not be empty".to_owned(),
            });
        }
        if self.capabilities.iter().collect::<BTreeSet<_>>().len() != self.capabilities.len() {
            return Err(ConfigError::Invalid {
                path: "provider_capabilities.capabilities".to_owned(),
                message: "capabilities must be unique".to_owned(),
            });
        }
        let mut operation_names = BTreeSet::new();
        for operation in &self.operations {
            if operation.name.is_empty() || !operation_names.insert(&operation.name) {
                return Err(ConfigError::Invalid {
                    path: "provider_capabilities.operations".to_owned(),
                    message: "operation names must be non-empty and unique".to_owned(),
                });
            }
            if operation.idempotent
                && matches!(
                    operation.effect_class,
                    EffectClass::PaidInference
                        | EffectClass::NonIdempotentWrite
                        | EffectClass::Production
                        | EffectClass::Credential
                )
            {
                return Err(ConfigError::Invalid {
                    path: format!("provider_capabilities.operations.{}", operation.name),
                    message: "effect class cannot be declared idempotent".to_owned(),
                });
            }
        }
        Ok(())
    }
}

pub fn resolve_role(
    config: &WorkbenchConfiguration,
    role_name: &str,
    available: &BTreeMap<String, ProviderCapabilities>,
) -> Result<ResolvedModel, ConfigError> {
    validate_identifier(role_name, "role")?;
    let role = config
        .roles
        .get(role_name)
        .ok_or_else(|| ConfigError::Invalid {
            path: "role".to_owned(),
            message: "unknown role".to_owned(),
        })?;
    let aliases = std::iter::once(&role.model).chain(role.fallback_models.iter());
    for (index, alias) in aliases.enumerate() {
        let model = &config.models[alias];
        let Some(capabilities) = available.get(&model.provider) else {
            continue;
        };
        capabilities.validate()?;
        let supported = capabilities
            .capabilities
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if capabilities.authentication == Authentication::Available
            && role
                .required_capabilities
                .iter()
                .all(|capability| supported.contains(capability))
        {
            return Ok(ResolvedModel {
                role: role_name.to_owned(),
                model_alias: alias.clone(),
                provider: model.provider.clone(),
                runtime_model: model.runtime_model.clone(),
                used_fallback: index > 0,
            });
        }
    }
    Err(ConfigError::CapabilityUnavailable {
        role: role_name.to_owned(),
    })
}
