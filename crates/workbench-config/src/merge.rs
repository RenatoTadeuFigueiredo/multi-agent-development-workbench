use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::{
    model::WorkbenchConfiguration,
    validate::{ConfigError, validate},
};

const TOP_LEVEL_KEYS: &[&str] = &[
    "version",
    "providers",
    "models",
    "roles",
    "tools",
    "data_sources",
    "mcp_servers",
    "workflows",
    "routing",
    "policies",
    "storage",
    "protocol",
];

#[derive(Clone, Debug)]
pub struct ConfigLayer {
    pub name: String,
    value: Value,
}

impl ConfigLayer {
    pub fn from_yaml(name: impl Into<String>, yaml: &str) -> Result<Self, ConfigError> {
        let name = name.into();
        let value: Value = serde_yaml_ng::from_str(yaml)
            .map_err(|error| ConfigError::Syntax(error.to_string()))?;
        validate_layer_value(&name, &value)?;
        Ok(Self { name, value })
    }

    pub fn from_configuration(
        name: impl Into<String>,
        config: &WorkbenchConfiguration,
    ) -> Result<Self, ConfigError> {
        let name = name.into();
        let value =
            serde_json::to_value(config).map_err(|error| ConfigError::Syntax(error.to_string()))?;
        Ok(Self { name, value })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedConfiguration {
    pub configuration: WorkbenchConfiguration,
    pub sources: Vec<String>,
}

pub fn resolve(layers: &[ConfigLayer]) -> Result<ResolvedConfiguration, ConfigError> {
    if layers.is_empty() {
        return Err(ConfigError::Invalid {
            path: "layers".to_owned(),
            message: "at least the built-in layer is required".to_owned(),
        });
    }
    let mut merged = Value::Object(Map::new());
    for layer in layers {
        merge_value(&mut merged, &layer.value, "");
    }
    let configuration: WorkbenchConfiguration =
        serde_json::from_value(merged).map_err(|error| ConfigError::Syntax(error.to_string()))?;
    validate(&configuration)?;
    Ok(ResolvedConfiguration {
        configuration,
        sources: layers.iter().map(|layer| layer.name.clone()).collect(),
    })
}

pub fn resolve_with_builtins(
    higher_precedence: &[ConfigLayer],
) -> Result<ResolvedConfiguration, ConfigError> {
    let mut layers = vec![ConfigLayer::from_configuration(
        "builtins",
        &WorkbenchConfiguration::safe_builtins(),
    )?];
    layers.extend_from_slice(higher_precedence);
    resolve(&layers)
}

fn merge_value(base: &mut Value, overlay: &Value, path: &str) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if let Some(existing) = base.get_mut(key) {
                    merge_value(existing, value, &child_path);
                } else {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
        (Value::Array(base), Value::Array(overlay)) if path == "policies.global_deny" => {
            for value in overlay {
                if !base.contains(value) {
                    base.push(value.clone());
                }
            }
        }
        (Value::String(base), Value::String(overlay)) if path == "policies.default_tool_mode" => {
            if policy_rank(overlay) > policy_rank(base) {
                overlay.clone_into(base);
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn policy_rank(mode: &str) -> u8 {
    match mode {
        "read-only" => 0,
        "approval-required" => 1,
        "denied" => 2,
        _ => u8::MAX,
    }
}

fn validate_layer_value(name: &str, value: &Value) -> Result<(), ConfigError> {
    let Value::Object(object) = value else {
        return Err(ConfigError::Invalid {
            path: name.to_owned(),
            message: "configuration layer must be a mapping".to_owned(),
        });
    };
    let allowed = TOP_LEVEL_KEYS.iter().copied().collect::<BTreeSet<_>>();
    for key in object.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ConfigError::Invalid {
                path: format!("{name}.{key}"),
                message: "unknown top-level field".to_owned(),
            });
        }
    }
    reject_nulls(name, value)?;
    if let Some(version) = object.get("version")
        && version.as_u64() != Some(1)
    {
        return Err(ConfigError::Invalid {
            path: format!("{name}.version"),
            message: "only schema version 1 is supported".to_owned(),
        });
    }
    Ok(())
}

fn reject_nulls(path: &str, value: &Value) -> Result<(), ConfigError> {
    match value {
        Value::Null if path.ends_with(".storage.retention_days") => Ok(()),
        Value::Null => Err(ConfigError::Invalid {
            path: path.to_owned(),
            message: "explicit null is not allowed here".to_owned(),
        }),
        Value::Object(object) => object
            .iter()
            .try_for_each(|(key, value)| reject_nulls(&format!("{path}.{key}"), value)),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .try_for_each(|(index, value)| reject_nulls(&format!("{path}.{index}"), value)),
        _ => Ok(()),
    }
}
