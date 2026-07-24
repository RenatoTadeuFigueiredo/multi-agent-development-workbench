use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{model::WorkbenchConfiguration, validate::ConfigError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSnapshot {
    pub schema_version: u32,
    pub sources: Vec<String>,
    pub content_hash: String,
    pub configuration: Value,
}

impl ConfigurationSnapshot {
    pub fn create(
        config: &WorkbenchConfiguration,
        sources: Vec<String>,
    ) -> Result<Self, ConfigError> {
        let mut value =
            serde_json::to_value(config).map_err(|error| ConfigError::Syntax(error.to_string()))?;
        redact(&mut value, None);
        let canonical = canonical_json(&value)?;
        let content_hash = blake3::hash(canonical.as_bytes()).to_hex().to_string();
        Ok(Self {
            schema_version: 1,
            sources,
            content_hash,
            configuration: value,
        })
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, ConfigError> {
    serde_json::to_string(value).map_err(|error| ConfigError::Syntax(error.to_string()))
}

fn redact(value: &mut Value, key: Option<&str>) {
    if matches!(
        key,
        Some("credential_ref" | "executable" | "url" | "env" | "headers" | "args")
    ) {
        *value = Value::String("[redacted]".to_owned());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                redact(value, Some(key));
            }
        }
        Value::Array(values) => {
            for value in values {
                redact(value, None);
            }
        }
        _ => {}
    }
}
