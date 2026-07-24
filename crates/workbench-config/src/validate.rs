use std::collections::BTreeSet;

use thiserror::Error;

use crate::model::{
    EffectClass, McpTransport, ProviderDriver, ProviderType, ToolKind, WorkbenchConfiguration,
};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid configuration syntax: {0}")]
    Syntax(String),
    #[error("invalid configuration value at {path}: {message}")]
    Invalid { path: String, message: String },
    #[error("configuration source is unsafe: {0}")]
    UnsafeSource(String),
    #[error("configuration I/O failed: {0}")]
    Io(String),
    #[error("lock operation failed: {0}")]
    Lock(String),
    #[error("provider capability unavailable for role {role}")]
    CapabilityUnavailable { role: String },
}

#[allow(clippy::too_many_lines)]
pub fn validate(config: &WorkbenchConfiguration) -> Result<(), ConfigError> {
    if config.version != 1 {
        return invalid("version", "only schema version 1 is supported");
    }
    if config.providers.is_empty() || config.models.is_empty() || config.roles.is_empty() {
        return invalid(
            "configuration",
            "providers, models, and roles must not be empty",
        );
    }
    if !(0.0..=1.0).contains(&config.routing.confidence_threshold) {
        return invalid("routing.confidence_threshold", "must be between 0 and 1");
    }
    if config.storage.retention_days == Some(0) {
        return invalid(
            "storage.retention_days",
            "must be at least 1 when configured",
        );
    }
    if config.protocol.max_frame_bytes != 8_388_608
        || config.protocol.max_client_queue_events != 1_024
        || config.protocol.max_client_queue_bytes != 8_388_608
        || config.protocol.cancellation_deadline_ms != 5_000
    {
        return invalid("protocol", "feature 001 protocol limits are fixed");
    }

    for (name, provider) in &config.providers {
        validate_identifier(name, &format!("providers.{name}"))?;
        if provider.kind == ProviderType::Api
            && (provider.credential_ref.is_none() || provider.privacy.is_none())
        {
            return invalid(
                &format!("providers.{name}"),
                "API providers require credential_ref and privacy",
            );
        }
        if provider.kind == ProviderType::Acp
            && provider.executable.as_deref().is_none_or(str::is_empty)
        {
            return invalid(
                &format!("providers.{name}.executable"),
                "ACP providers require an explicit executable",
            );
        }
        if provider.kind == ProviderType::SubscriptionCli {
            if !matches!(
                provider.driver,
                Some(ProviderDriver::ClaudeCode | ProviderDriver::Codex)
            ) {
                return invalid(
                    &format!("providers.{name}.driver"),
                    "subscription CLI providers require a supported driver",
                );
            }
            if provider.executable.as_deref().is_none_or(str::is_empty) {
                return invalid(
                    &format!("providers.{name}.executable"),
                    "subscription CLI providers require an explicit executable",
                );
            }
        } else if provider.driver.is_some() {
            return invalid(
                &format!("providers.{name}.driver"),
                "driver is only valid for subscription CLI providers",
            );
        }
        if let Some(reference) = &provider.credential_ref {
            validate_credential_reference(reference, name)?;
        }
    }

    for (name, model) in &config.models {
        validate_identifier(name, &format!("models.{name}"))?;
        if !config.providers.contains_key(&model.provider) {
            return invalid(
                &format!("models.{name}.provider"),
                "references an unknown provider",
            );
        }
        if model.runtime_model.is_empty() || model.runtime_model.len() > 255 {
            return invalid(
                &format!("models.{name}.runtime_model"),
                "must contain 1 to 255 bytes",
            );
        }
    }

    for (name, server) in &config.mcp_servers {
        validate_identifier(name, &format!("mcp_servers.{name}"))?;
        validate_digest(&server.sha256, &format!("mcp_servers.{name}.sha256"))?;
        if server.version.is_empty() {
            return invalid(&format!("mcp_servers.{name}.version"), "must not be empty");
        }
        match server.transport {
            McpTransport::Stdio => {
                if server.executable.as_deref().is_none_or(str::is_empty) {
                    return invalid(
                        &format!("mcp_servers.{name}.executable"),
                        "stdio MCP servers require an absolute executable path",
                    );
                }
                if server.url.is_some() {
                    return invalid(
                        &format!("mcp_servers.{name}.url"),
                        "stdio MCP servers cannot declare a URL",
                    );
                }
                if !server.headers.is_empty() {
                    return invalid(
                        &format!("mcp_servers.{name}.headers"),
                        "stdio MCP servers cannot declare HTTP headers",
                    );
                }
                for (key, handle) in &server.env {
                    validate_secret_handle(
                        handle,
                        &format!("mcp_servers.{name}.env.{key}"),
                    )?;
                }
            }
            McpTransport::Http => {
                if server.url.as_deref().is_none_or(str::is_empty) {
                    return invalid(
                        &format!("mcp_servers.{name}.url"),
                        "HTTP MCP servers require an absolute URL",
                    );
                }
                if server.executable.is_some() || !server.args.is_empty() || !server.env.is_empty()
                {
                    return invalid(
                        &format!("mcp_servers.{name}"),
                        "HTTP MCP servers cannot declare stdio launch fields",
                    );
                }
                if let Some(limit) = server.max_response_bytes
                    && !(1..=8_388_608).contains(&limit)
                {
                    return invalid(
                        &format!("mcp_servers.{name}.max_response_bytes"),
                        "must be between 1 and 8388608",
                    );
                }
                for (key, handle) in &server.headers {
                    validate_secret_handle(
                        handle,
                        &format!("mcp_servers.{name}.headers.{key}"),
                    )?;
                }
            }
        }
    }

    for (name, tool) in &config.tools {
        validate_identifier(name, &format!("tools.{name}"))?;
        if tool.operations.is_empty() {
            return invalid(&format!("tools.{name}.operations"), "must not be empty");
        }
        match tool.kind {
            ToolKind::Mcp => {
                let Some(server) = &tool.mcp_server else {
                    return invalid(&format!("tools.{name}"), "MCP tool requires mcp_server");
                };
                if !config.mcp_servers.contains_key(server) {
                    return invalid(
                        &format!("tools.{name}.mcp_server"),
                        "references an unknown MCP server",
                    );
                }
            }
            ToolKind::Builtin if tool.mcp_server.is_some() => {
                return invalid(
                    &format!("tools.{name}.mcp_server"),
                    "built-in tool cannot reference an MCP server",
                );
            }
            ToolKind::Builtin => {}
        }
        let mut operations = BTreeSet::new();
        for operation in &tool.operations {
            validate_identifier(
                &operation.name,
                &format!("tools.{name}.operations.{}", operation.name),
            )?;
            if !operations.insert(&operation.name) {
                return invalid(
                    &format!("tools.{name}.operations"),
                    "operation names must be unique",
                );
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
                return invalid(
                    &format!("tools.{name}.operations.{}.idempotent", operation.name),
                    "effect class cannot be declared idempotent",
                );
            }
        }
    }

    for (name, source) in &config.data_sources {
        validate_identifier(name, &format!("data_sources.{name}"))?;
        let Some(tool) = config.tools.get(&source.tool) else {
            return invalid(
                &format!("data_sources.{name}.tool"),
                "references an unknown tool",
            );
        };
        let Some(operation) = tool
            .operations
            .iter()
            .find(|operation| operation.name == source.operation)
        else {
            return invalid(
                &format!("data_sources.{name}.operation"),
                "references an unknown operation",
            );
        };
        if operation.effect_class != EffectClass::IdempotentRead || !operation.idempotent {
            return invalid(
                &format!("data_sources.{name}"),
                "data source must resolve to an explicitly idempotent read",
            );
        }
    }

    for (name, role) in &config.roles {
        validate_identifier(name, &format!("roles.{name}"))?;
        if !config.models.contains_key(&role.model) {
            return invalid(
                &format!("roles.{name}.model"),
                "references an unknown model",
            );
        }
        validate_unique(&role.tools, &format!("roles.{name}.tools"))?;
        validate_unique(&role.data_sources, &format!("roles.{name}.data_sources"))?;
        validate_unique(
            &role.fallback_models,
            &format!("roles.{name}.fallback_models"),
        )?;
        if role
            .required_capabilities
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != role.required_capabilities.len()
        {
            return invalid(
                &format!("roles.{name}.required_capabilities"),
                "values must be unique",
            );
        }
        for tool in &role.tools {
            if !config.tools.contains_key(tool) {
                return invalid(&format!("roles.{name}.tools"), "references an unknown tool");
            }
        }
        for source in &role.data_sources {
            if !config.data_sources.contains_key(source) {
                return invalid(
                    &format!("roles.{name}.data_sources"),
                    "references an unknown data source",
                );
            }
        }
        for model in &role.fallback_models {
            if !config.models.contains_key(model) {
                return invalid(
                    &format!("roles.{name}.fallback_models"),
                    "references an unknown model",
                );
            }
        }
    }

    if !config.roles.contains_key(&config.routing.default_role) {
        return invalid("routing.default_role", "references an unknown role");
    }
    for denied in &config.policies.global_deny {
        if !config.tools.contains_key(denied) {
            return invalid("policies.global_deny", "references an unknown tool");
        }
    }

    for (name, workflow) in &config.workflows {
        validate_identifier(name, &format!("workflows.{name}"))?;
        if workflow.steps.is_empty() {
            return invalid(&format!("workflows.{name}.steps"), "must not be empty");
        }
        let ids = workflow
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != workflow.steps.len() {
            return invalid(
                &format!("workflows.{name}.steps"),
                "step identifiers must be unique",
            );
        }
        for step in &workflow.steps {
            validate_identifier(&step.id, &format!("workflows.{name}.steps"))?;
            if !config.roles.contains_key(&step.role) {
                return invalid(
                    &format!("workflows.{name}.steps.{}.role", step.id),
                    "references an unknown role",
                );
            }
            if let Some(target) = &step.on_findings
                && !ids.contains(target.as_str())
            {
                return invalid(
                    &format!("workflows.{name}.steps.{}.on_findings", step.id),
                    "references an unknown workflow step",
                );
            }
            if step.max_iterations == Some(0) {
                return invalid(
                    &format!("workflows.{name}.steps.{}.max_iterations", step.id),
                    "must be at least 1",
                );
            }
            validate_unique(
                &step.tools,
                &format!("workflows.{name}.steps.{}.tools", step.id),
            )?;
            for tool in &step.tools {
                if !config.tools.contains_key(tool) {
                    return invalid(
                        &format!("workflows.{name}.steps.{}.tools", step.id),
                        "references an unknown tool",
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn validate_identifier(value: &str, path: &str) -> Result<(), ConfigError> {
    let mut chars = value.chars();
    let first_valid = chars
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let rest_valid = chars.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if first_valid && rest_valid && value.len() <= 63 {
        Ok(())
    } else {
        invalid(path, "must match ^[a-z][a-z0-9-]{0,62}$")
    }
}

pub fn validate_digest(value: &str, path: &str) -> Result<(), ConfigError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        invalid(path, "must be a lowercase 64-character hexadecimal digest")
    }
}

fn validate_credential_reference(reference: &str, provider: &str) -> Result<(), ConfigError> {
    validate_secret_handle(
        reference,
        &format!("providers.{provider}.credential_ref"),
    )
}

fn validate_secret_handle(reference: &str, path: &str) -> Result<(), ConfigError> {
    let accepted_prefix = ["platform:", "keychain:", "secret-service:"]
        .iter()
        .any(|prefix| reference.starts_with(prefix));
    let suffix = reference.split_once(':').map_or("", |(_, value)| value);
    if accepted_prefix
        && !suffix.is_empty()
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
    {
        Ok(())
    } else {
        invalid(path, "must be an opaque platform credential reference")
    }
}

fn validate_unique(values: &[String], path: &str) -> Result<(), ConfigError> {
    if values.iter().collect::<BTreeSet<_>>().len() == values.len() {
        Ok(())
    } else {
        invalid(path, "values must be unique")
    }
}

fn invalid<T>(path: &str, message: &str) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid {
        path: path.to_owned(),
        message: message.to_owned(),
    })
}
