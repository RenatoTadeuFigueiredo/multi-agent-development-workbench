use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use tempfile::TempDir;
use workbench_config::{
    AdapterInput, ConfigError, ConfigLayer, ConfigurationSnapshot, WorkbenchConfiguration,
    lock::{ACP_PROTOCOL, CLAUDE_CODE_STREAM_PROTOCOL, WorkbenchLock},
    merge::resolve_with_builtins,
    model::{Capability, Model, Provider, ProviderDriver, ProviderType, Role},
    preflight::{Authentication, ProviderCapabilities, resolve_role},
    validate::validate,
};

#[test]
fn builtins_fixture_is_exact_and_valid() {
    let fixture = ConfigLayer::from_yaml("fixture", include_str!("../fixtures/builtins.yaml"))
        .expect("fixture parses");
    let resolved = workbench_config::merge::resolve(&[fixture]).expect("fixture resolves");
    assert_eq!(
        resolved.configuration,
        WorkbenchConfiguration::safe_builtins()
    );
    validate(&resolved.configuration).expect("built-ins validate");
}

#[test]
fn higher_precedence_layer_wins_and_invalid_reference_does_not_fall_back() {
    let valid = ConfigLayer::from_yaml(
        "repository",
        r"
models:
  fake-default:
    provider: fake
    runtime_model: repository-model
",
    )
    .expect("repository layer parses");
    let session = ConfigLayer::from_yaml(
        "session",
        r"
models:
  fake-default:
    provider: fake
    runtime_model: session-model
",
    )
    .expect("session layer parses");
    let resolved = resolve_with_builtins(&[valid, session]).expect("layers resolve");
    assert_eq!(
        resolved.configuration.models["fake-default"].runtime_model,
        "session-model"
    );

    let invalid = ConfigLayer::from_yaml(
        "session",
        r"
models:
  fake-default:
    provider: missing
",
    )
    .expect("partial layer syntax is valid");
    assert!(resolve_with_builtins(&[invalid]).is_err());
}

#[test]
fn repository_cannot_remove_an_existing_global_deny() {
    let mut builtins = WorkbenchConfiguration::safe_builtins();
    builtins.tools.insert(
        "terminal".to_owned(),
        workbench_config::model::Tool {
            kind: workbench_config::model::ToolKind::Builtin,
            mcp_server: None,
            operations: vec![workbench_config::model::Operation {
                name: "run".to_owned(),
                effect_class: workbench_config::model::EffectClass::NonIdempotentWrite,
                idempotent: false,
                material_cost: false,
                approval: workbench_config::model::ApprovalMode::Policy,
            }],
        },
    );
    builtins.policies.global_deny.push("terminal".to_owned());
    let base = ConfigLayer::from_configuration("user", &builtins).expect("base serializes");
    let repository = ConfigLayer::from_yaml("repository", "policies:\n  global_deny: []\n")
        .expect("repository parses");
    let resolved =
        workbench_config::merge::resolve(&[base, repository]).expect("configuration resolves");
    assert_eq!(resolved.configuration.policies.global_deny, ["terminal"]);
}

#[test]
fn later_layers_cannot_widen_the_default_tool_mode() {
    let user = ConfigLayer::from_yaml("user", "policies:\n  default_tool_mode: denied\n")
        .expect("user policy");
    let repository =
        ConfigLayer::from_yaml("repository", "policies:\n  default_tool_mode: read-only\n")
            .expect("repository policy");
    let session = ConfigLayer::from_yaml(
        "session",
        "policies:\n  default_tool_mode: approval-required\n",
    )
    .expect("session policy");

    let resolved =
        resolve_with_builtins(&[user, repository, session]).expect("monotonic policies resolve");

    assert_eq!(
        resolved.configuration.policies.default_tool_mode,
        workbench_config::model::DefaultToolMode::Denied
    );
}

#[test]
fn zero_day_retention_is_rejected() {
    let layer = ConfigLayer::from_yaml("repository", "storage:\n  retention_days: 0\n")
        .expect("retention layer parses");

    assert!(resolve_with_builtins(&[layer]).is_err());
}

#[test]
fn snapshot_redacts_references_and_is_deterministic() {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "service".to_owned(),
        Provider {
            kind: ProviderType::Api,
            driver: None,
            executable: Some("/private/provider".to_owned()),
            credential_ref: Some("platform:service".to_owned()),
            privacy: Some(workbench_config::model::Privacy {
                zero_data_retention: true,
                data_collection: workbench_config::model::DataCollection::Deny,
            }),
            base_url: None,
        },
    );
    let first = ConfigurationSnapshot::create(&configuration, vec!["builtins".to_owned()])
        .expect("snapshot");
    let second = ConfigurationSnapshot::create(&configuration, vec!["builtins".to_owned()])
        .expect("snapshot");
    assert_eq!(first, second);
    let encoded = serde_json::to_string(&first).expect("snapshot serializes");
    assert!(!encoded.contains("platform:service"));
    assert!(!encoded.contains("/private/provider"));
}

#[test]
fn lock_and_hash_are_byte_deterministic() {
    let configuration = WorkbenchConfiguration::safe_builtins();
    let snapshot = ConfigurationSnapshot::create(&configuration, vec!["builtins".to_owned()])
        .expect("snapshot");
    let first = WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new())
        .expect("repository lock");
    let second = WorkbenchLock::repository(&configuration, &snapshot, &BTreeMap::new())
        .expect("repository lock");
    assert_eq!(
        serde_json::to_vec(&first).expect("serialize"),
        serde_json::to_vec(&second).expect("serialize")
    );
    assert_eq!(first.hash().expect("hash"), second.hash().expect("hash"));
    let session = WorkbenchLock::session(&first, &configuration, &snapshot).expect("session lock");
    assert_eq!(
        session.base_lock_hash,
        Some(first.hash().expect("base hash"))
    );
    first.verify().expect("valid lock");
    session.verify().expect("valid linked lock");
    session
        .verify_linked_to(&first)
        .expect("session links to base");
}

#[test]
fn acp_providers_require_an_explicit_executable() {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "grok".to_owned(),
        Provider {
            kind: ProviderType::Acp,
            driver: None,
            executable: None,
            credential_ref: None,
            privacy: None,
            base_url: None,
        },
    );

    let error = validate(&configuration).expect_err("ACP executable is required");

    assert!(matches!(
        error,
        ConfigError::Invalid { path, .. } if path == "providers.grok.executable"
    ));
}

#[test]
fn acp_adapter_input_pins_protocol_version_and_executable_digest() {
    let directory =
        TempDir::new_in(std::env::current_dir().expect("current directory")).expect("temporary");
    let executable = directory.path().join("fake-acp");
    fs::write(&executable, b"fake-acp-v1").expect("fake executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("executable permissions");
    let executable = executable.canonicalize().expect("canonical executable");
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "grok".to_owned(),
        Provider {
            kind: ProviderType::Acp,
            driver: None,
            executable: Some(executable.to_string_lossy().into_owned()),
            credential_ref: None,
            privacy: None,
            base_url: None,
        },
    );
    validate(&configuration).expect("ACP configuration");
    let snapshot =
        ConfigurationSnapshot::create(&configuration, vec!["test".to_owned()]).expect("snapshot");
    let inputs = BTreeMap::from([(
        "grok".to_owned(),
        AdapterInput::acp(&executable, "0.2.7").expect("adapter input"),
    )]);

    let lock =
        WorkbenchLock::repository(&configuration, &snapshot, &inputs).expect("repository lock");

    assert_eq!(lock.adapters["grok"].protocol, ACP_PROTOCOL);
    assert_eq!(lock.adapters["grok"].version, "0.2.7");
    assert_eq!(lock.adapters["grok"].executable_sha256.len(), 64);
    lock.verify_executables(&inputs)
        .expect("executable matches lock");

    fs::write(&executable, b"fake-acp-v2").expect("updated executable");
    assert!(matches!(
        WorkbenchLock::repository(&configuration, &snapshot, &inputs),
        Err(ConfigError::Lock(message)) if message.contains("changed after identity capture")
    ));
}

#[test]
fn claude_subscription_provider_requires_driver_and_pins_stream_protocol() {
    let directory =
        TempDir::new_in(std::env::current_dir().expect("current directory")).expect("temporary");
    let executable = directory.path().join("fake-claude");
    fs::write(&executable, b"fake-claude-v1").expect("fake executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("executable permissions");
    let executable = executable.canonicalize().expect("canonical executable");
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "claude".to_owned(),
        Provider {
            kind: ProviderType::SubscriptionCli,
            driver: Some(ProviderDriver::ClaudeCode),
            executable: Some(executable.to_string_lossy().into_owned()),
            credential_ref: None,
            privacy: None,
            base_url: None,
        },
    );
    validate(&configuration).expect("Claude subscription configuration");
    let snapshot =
        ConfigurationSnapshot::create(&configuration, vec!["test".to_owned()]).expect("snapshot");
    let inputs = BTreeMap::from([(
        "claude".to_owned(),
        AdapterInput::claude_code(&executable, "2.1.218").expect("adapter input"),
    )]);

    let lock =
        WorkbenchLock::repository(&configuration, &snapshot, &inputs).expect("repository lock");

    assert_eq!(
        lock.adapters["claude"].protocol,
        CLAUDE_CODE_STREAM_PROTOCOL
    );
    assert_eq!(lock.adapters["claude"].version, "2.1.218");
    lock.verify_configured_executables(&configuration)
        .expect("configured executable matches lock");
}

#[test]
fn subscription_provider_without_driver_or_executable_is_rejected() {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "claude".to_owned(),
        Provider {
            kind: ProviderType::SubscriptionCli,
            driver: None,
            executable: None,
            credential_ref: None,
            privacy: None,
            base_url: None,
        },
    );

    let error = validate(&configuration).expect_err("driver is required");
    assert!(matches!(
        error,
        ConfigError::Invalid { path, .. } if path == "providers.claude.driver"
    ));
}

#[test]
fn acp_adapter_input_rejects_symlinks_and_unsafe_permissions() {
    let directory =
        TempDir::new_in(std::env::current_dir().expect("current directory")).expect("temporary");
    let target = directory.path().join("fake-acp");
    fs::write(&target, b"fake-acp-v1").expect("fake executable");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
        .expect("executable permissions");
    let linked = directory.path().join("linked-acp");
    symlink(&target, &linked).expect("symbolic executable");

    assert!(matches!(
        AdapterInput::acp("relative/fake-acp", "0.2.7"),
        Err(ConfigError::UnsafeSource(_))
    ));
    assert!(matches!(
        AdapterInput::acp(&linked, "0.2.7"),
        Err(ConfigError::UnsafeSource(_))
    ));

    fs::set_permissions(&target, fs::Permissions::from_mode(0o722))
        .expect("unsafe executable permissions");
    assert!(matches!(
        AdapterInput::acp(&target, "0.2.7"),
        Err(ConfigError::UnsafeSource(_))
    ));
}

#[test]
fn preflight_chooses_the_first_compatible_fallback() {
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    configuration.providers.insert(
        "fallback".to_owned(),
        Provider {
            kind: ProviderType::Fake,
            driver: None,
            executable: None,
            credential_ref: None,
            privacy: None,
            base_url: None,
        },
    );
    configuration.models.insert(
        "fallback-model".to_owned(),
        Model {
            provider: "fallback".to_owned(),
            runtime_model: "fallback-v1".to_owned(),
        },
    );
    configuration.roles.insert(
        "reviewer".to_owned(),
        Role {
            model: "fake-default".to_owned(),
            tools: Vec::new(),
            data_sources: Vec::new(),
            required_capabilities: vec![Capability::StructuredOutput],
            fallback_models: vec!["fallback-model".to_owned()],
        },
    );
    let capabilities = BTreeMap::from([
        ("fake".to_owned(), provider_capabilities(Vec::new())),
        (
            "fallback".to_owned(),
            provider_capabilities(vec![Capability::StructuredOutput]),
        ),
    ]);
    let resolved = resolve_role(&configuration, "reviewer", &capabilities).expect("fallback");
    assert_eq!(resolved.provider, "fallback");
    assert!(resolved.used_fallback);
}

fn provider_capabilities(capabilities: Vec<Capability>) -> ProviderCapabilities {
    ProviderCapabilities {
        adapter_id: "fake".to_owned(),
        adapter_version: "1".to_owned(),
        protocol: "test".to_owned(),
        authentication: Authentication::Available,
        capabilities,
        context_window_tokens: None,
        operations: vec![workbench_config::preflight::ProviderOperation {
            name: "prompt".to_owned(),
            effect_class: workbench_config::model::EffectClass::PaidInference,
            idempotent: false,
            material_cost: false,
            approval: workbench_config::model::ApprovalMode::Never,
        }],
    }
}
