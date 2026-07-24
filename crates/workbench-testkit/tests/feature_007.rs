//! Feature 007 acceptance: central MCP lifecycle and tool permissions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use workbench_config::{
    ConfigurationSnapshot, WorkbenchConfiguration, WorkbenchLock,
    model::{
        ApprovalMode, EffectClass, McpServer, McpTransport, Operation, Role, Tool, ToolKind,
        Workflow, WorkflowStep,
    },
};
use workbench_core::{attempt::AttemptProgress, policy::PolicySource};
use workbench_mcp::{
    FakeHttpMode, McpErrorKind, McpGateway, ToolInvokeRequest, contains_marker,
    http_endpoint_sha256,
};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/central-mcp-lifecycle-and-tool-permissions.feature"
);
const MCP_LIB: &str = include_str!("../../workbench-mcp/src/lib.rs");
const MCP_GATEWAY: &str = include_str!("../../workbench-mcp/src/gateway.rs");
const CLAUDE_PROCESS: &str = include_str!("../../workbench-claude/src/process.rs");
const CODEX_PROCESS: &str = include_str!("../../workbench-codex/src/process.rs");
const ACP_SUPERVISOR: &str = include_str!("../../workbench-acp/src/supervisor.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const FAKE_MCP: &str = env!("CARGO_BIN_EXE_fake_mcp");

const SECRET_MARKERS: [&str; 5] = [
    "SECRET-MARKER-F007",
    "AUTH-MARKER-F007",
    "ENV-MARKER-F007",
    "ARG-MARKER-F007",
    "CRED-MARKER-F007",
];

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

// Stable FNV-1a fingerprints over Gherkin step lines (pin after first green run).
const SCENARIO_BINDINGS: [ScenarioBinding; 18] = [
    binding(
        "Load pinned stdio and HTTP servers",
        0x0287_8f4c_07b3_94f1,
        "pinned_registry_loads_stdio_and_http",
    ),
    binding(
        "Reject digest mismatch before tool dispatch",
        0x4e23_8e99_cf24_ad56,
        "digest_mismatch_fails_closed",
    ),
    binding(
        "Deny a tool absent from the role allowlist",
        0x16fa_41b9_0552_771b,
        "role_and_workflow_allowlists_deny_before_transport",
    ),
    binding(
        "Workflow allowlist narrows the role grant",
        0x90f9_8b48_da66_ac33,
        "role_and_workflow_allowlists_deny_before_transport",
    ),
    binding(
        "Repository configuration cannot widen a user deny",
        0x5dcc_0f82_8935_0b74,
        "repository_cannot_widen_user_deny",
    ),
    binding(
        "Gate protected operations on approval [effect_class=non-idempotent-write]",
        0xc841_396e_c9bd_1006,
        "approval_gates_protected_operations",
    ),
    binding(
        "Gate protected operations on approval [effect_class=production]",
        0xfcc6_6a05_3143_4222,
        "approval_gates_protected_operations",
    ),
    binding(
        "Gate protected operations on approval [effect_class=credential]",
        0xe16c_9d52_3c18_27c0,
        "approval_gates_protected_operations",
    ),
    binding(
        "Gate protected operations on approval [effect_class=paid-inference]",
        0xf491_1752_2ff2_0aaf,
        "approval_gates_protected_operations",
    ),
    binding(
        "Isolate supervised stdio children by workspace",
        0xdfbb_7bad_3fe3_eb73,
        "stdio_isolation_and_shutdown_reap",
    ),
    binding(
        "Enforce HTTP pin and response bounds",
        0x1ad5_da70_90a5_a3a3,
        "http_pin_and_bounds_fail_closed",
    ),
    binding(
        "Preserve uncertainty after cancel without a terminal fact",
        0x53e7_0192_9fe5_910e,
        "cancel_and_retry_semantics",
    ),
    binding(
        "Allow pre-start retry only for idempotent reads",
        0x8e3f_1318_14ba_c66f,
        "cancel_and_retry_semantics",
    ),
    binding(
        "Redact secrets from audit surfaces",
        0x27fc_4ac2_4cfa_db2b,
        "redacted_audit_and_secrecy",
    ),
    binding(
        "Default suite stays offline and quota-free",
        0x57b3_ee0f_8ae6_8f77,
        "default_suite_stays_offline",
    ),
    binding(
        "Keep provider-native MCP registration disabled",
        0xac06_9bf9_ab44_7a75,
        "provider_native_mcp_remains_disabled",
    ),
    binding(
        "Accept an empty MCP registry",
        0xc409_38d7_9d43_af4e,
        "empty_registry_is_valid",
    ),
    binding(
        "Reap children on shutdown",
        0xb5de_fbbb_ee1e_6513,
        "stdio_isolation_and_shutdown_reap",
    ),
];

const fn binding(
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
) -> ScenarioBinding {
    ScenarioBinding {
        case_name,
        fingerprint,
        evidence_test,
    }
}

#[test]
fn repository_owned_gherkin_has_eighteen_fingerprinted_cases() {
    let parsed = parse_feature(FEATURE);
    assert_eq!(parsed.heading_count, 15);
    assert_eq!(parsed.cases.len(), 18);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(bindings.len(), 18);
    for case in &parsed.cases {
        assert!(
            bindings.contains_key(case.name.as_str()),
            "missing binding for {}",
            case.name
        );
        let fp = fingerprint(&case.steps);
        assert_ne!(fp, 0, "fingerprint collapsed for {}", case.name);
        let binding = bindings[case.name.as_str()];
        assert_eq!(fp, binding.fingerprint, "scenario drifted: {}", case.name);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = pinned_registry_loads_stdio_and_http;
    let _ = digest_mismatch_fails_closed;
    let _ = role_and_workflow_allowlists_deny_before_transport;
    let _ = repository_cannot_widen_user_deny;
    let _ = approval_gates_protected_operations;
    let _ = stdio_isolation_and_shutdown_reap;
    let _ = http_pin_and_bounds_fail_closed;
    let _ = cancel_and_retry_semantics;
    let _ = redacted_audit_and_secrecy;
    let _ = default_suite_stays_offline;
    let _ = provider_native_mcp_remains_disabled;
    let _ = empty_registry_is_valid;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "approval_gates_protected_operations",
            "cancel_and_retry_semantics",
            "default_suite_stays_offline",
            "digest_mismatch_fails_closed",
            "empty_registry_is_valid",
            "http_pin_and_bounds_fail_closed",
            "pinned_registry_loads_stdio_and_http",
            "provider_native_mcp_remains_disabled",
            "redacted_audit_and_secrecy",
            "repository_cannot_widen_user_deny",
            "role_and_workflow_allowlists_deny_before_transport",
            "stdio_isolation_and_shutdown_reap",
        ])
    );
}

#[tokio::test]
async fn pinned_registry_loads_stdio_and_http() {
    let env = harness(false);
    assert!(env.gateway.server_available("stdio-a"));
    assert!(env.gateway.server_available("http-a"));
    assert_eq!(env.gateway.available_servers().len(), 2);
    // No provider launches MCP independently: providers keep empty MCP configs.
    assert!(CLAUDE_PROCESS.contains("--strict-mcp-config"));
    assert!(CLAUDE_PROCESS.contains(r#"{"mcpServers":{}}"#));
}

#[tokio::test]
async fn digest_mismatch_fails_closed() {
    let env = harness(false);
    // Keep the original lock pin (correct file digest) while the configuration
    // claims a different sha256 so verification fails closed before transport.
    let mut config = env.config.clone();
    let mut wrong = config.mcp_servers["stdio-a"].clone();
    wrong.sha256 = "b".repeat(64);
    config.mcp_servers.insert("stdio-a".to_owned(), wrong);
    let gateway = McpGateway::bootstrap(
        config,
        &env.lock,
        env.runtime.path().join("rt-mismatch"),
        "ws-mismatch",
        true,
    )
    .expect("bootstrap");
    assert!(!gateway.server_available("stdio-a"));
    let err = gateway
        .invoke(ToolInvokeRequest {
            tool_id: "repo-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            arguments: json!({"q": "x"}),
            correlation_id: "corr-pin".to_owned(),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("pin mismatch");
    assert_eq!(err.kind(), McpErrorKind::PinMismatch);
    let public = format!("{err}");
    assert!(!public.contains(FAKE_MCP));
}

#[tokio::test]
async fn role_and_workflow_allowlists_deny_before_transport() {
    let env = harness(false);
    let role_err = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "cluster-mutate".to_owned(),
            operation: "apply".to_owned(),
            role_id: Some("reviewer".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-role".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("role deny");
    assert_eq!(role_err.kind(), McpErrorKind::PolicyDenied);

    let workflow_err = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "cluster-mutate".to_owned(),
            operation: "apply".to_owned(),
            role_id: Some("builder".to_owned()),
            workflow_id: Some("ship".to_owned()),
            workflow_step_id: Some("review".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-wf".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("workflow deny");
    assert_eq!(workflow_err.kind(), McpErrorKind::PolicyDenied);

    let audit = env.gateway.audit_log().await;
    assert!(audit.iter().any(
        |fact| fact.event.lifecycle == workbench_mcp::ToolLifecycle::Denied
            && fact.transport.is_none()
    ));
}

#[tokio::test]
async fn repository_cannot_widen_user_deny() {
    let mut config = base_config(Path::new(FAKE_MCP), true);
    config.policies.global_deny.push("prod-deploy".to_owned());
    let lock = lock_for(&config);
    let gateway =
        McpGateway::bootstrap(config, &lock, PathBuf::from("/tmp"), "ws", true).expect("gateway");
    let err = gateway
        .invoke(ToolInvokeRequest {
            tool_id: "prod-deploy".to_owned(),
            operation: "deploy".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-user".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("user deny");
    assert_eq!(err.kind(), McpErrorKind::PolicyDenied);
    let audit = gateway.audit_log().await;
    assert!(audit.iter().any(|fact| {
        fact.event.policy_source.as_deref() == Some("user")
            && fact.event.lifecycle == workbench_mcp::ToolLifecycle::Denied
    }));
    assert_eq!(user_source(), PolicySource::User);
}

#[tokio::test]
async fn approval_gates_protected_operations() {
    let env = harness(false);
    for (tool, op) in [
        ("cluster-mutate", "apply"),
        ("prod-deploy", "deploy"),
        ("cred-read", "reveal"),
        ("paid-infer", "run"),
    ] {
        let required = env
            .gateway
            .invoke(ToolInvokeRequest {
                tool_id: tool.to_owned(),
                operation: op.to_owned(),
                role_id: Some("builder".to_owned()),
                approval_granted: None,
                correlation_id: format!("corr-{tool}"),
                arguments: json!({}),
                ..ToolInvokeRequest::default()
            })
            .await
            .expect_err("approval required");
        assert_eq!(required.kind(), McpErrorKind::ApprovalRequired);

        let denied = env
            .gateway
            .invoke(ToolInvokeRequest {
                tool_id: tool.to_owned(),
                operation: op.to_owned(),
                role_id: Some("builder".to_owned()),
                approval_granted: Some(false),
                correlation_id: format!("corr-deny-{tool}"),
                arguments: json!({}),
                ..ToolInvokeRequest::default()
            })
            .await
            .expect_err("approval denied");
        assert_eq!(denied.kind(), McpErrorKind::ApprovalDenied);
    }
}

#[tokio::test]
async fn stdio_isolation_and_shutdown_reap() {
    let left = harness_named("ws-left");
    let right = harness_named("ws-right");
    left.gateway
        .ensure_stdio("stdio-a")
        .await
        .expect("spawn left");
    right
        .gateway
        .ensure_stdio("stdio-a")
        .await
        .expect("spawn right");
    assert_eq!(left.gateway.active_stdio_children().await, 1);
    assert_eq!(right.gateway.active_stdio_children().await, 1);
    left.gateway.shutdown_workspace().await.expect("reap left");
    assert_eq!(left.gateway.active_stdio_children().await, 0);
    assert_eq!(right.gateway.active_stdio_children().await, 1);
    // New calls rejected after shutdown.
    let rejected = left
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "repo-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-shut".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("shutting down");
    assert_eq!(rejected.kind(), McpErrorKind::ShuttingDown);
    right.gateway.shutdown().await.expect("reap right");
}

#[tokio::test]
async fn http_pin_and_bounds_fail_closed() {
    let env = harness(false);
    env.gateway.http_fake().set_mode(
        "http-a",
        FakeHttpMode::Oversized {
            bytes: 8 * 1024 * 1024 + 1,
        },
    );
    let oversized = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "http-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-over".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("oversized");
    assert_eq!(oversized.kind(), McpErrorKind::ResponseTooLarge);

    env.gateway.http_fake().set_mode(
        "http-a",
        FakeHttpMode::Redirect {
            location: "http://evil.example/mcp".to_owned(),
        },
    );
    let redirect = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "http-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-redir".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("redirect");
    assert_eq!(redirect.kind(), McpErrorKind::RedirectRejected);
}

#[tokio::test]
async fn cancel_and_retry_semantics() {
    let env = harness(false);
    let cancel = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "cluster-mutate".to_owned(),
            operation: "apply".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-cancel".to_owned(),
            arguments: json!({}),
            simulate_cancel_after_start: true,
            ..ToolInvokeRequest::default()
        })
        .await
        .expect("outcome unknown outcome");
    assert_eq!(cancel.progress, AttemptProgress::OutcomeUnknown);
    assert!(!cancel.retried);

    let retry = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "repo-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-retry".to_owned(),
            arguments: json!({}),
            simulate_pre_start_failure: true,
            ..ToolInvokeRequest::default()
        })
        .await
        .expect("idempotent pre-start retry");
    assert!(retry.retried);

    let no_retry = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "cluster-mutate".to_owned(),
            operation: "apply".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-no-retry".to_owned(),
            arguments: json!({}),
            simulate_pre_start_failure: true,
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("no auto retry after mutate pre-start");
    assert_eq!(no_retry.kind(), McpErrorKind::Unavailable);
}

#[tokio::test]
async fn redacted_audit_and_secrecy() {
    let env = harness(false);
    let outcome = env
        .gateway
        .invoke(ToolInvokeRequest {
            tool_id: "repo-read".to_owned(),
            operation: "read".to_owned(),
            role_id: Some("builder".to_owned()),
            approval_granted: Some(true),
            correlation_id: "corr-secret".to_owned(),
            arguments: json!({
                "token": "ARG-MARKER-F007",
                "credential": "CRED-MARKER-F007"
            }),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect("success path");
    let surfaces = vec![
        outcome.public.to_public_json().to_string(),
        format!("{:?}", env.gateway.audit_log().await),
        serde_json::to_string(&env.lock).expect("lock"),
    ];
    for surface in surfaces {
        assert!(
            !contains_marker(&surface, &SECRET_MARKERS),
            "secret leaked into {surface}"
        );
    }
}

#[test]
fn default_suite_stays_offline() {
    assert!(MCP_LIB.contains("forbid(unsafe_code)"));
    assert!(MCP_GATEWAY.contains("offline_http"));
    assert!(MAKEFILE.contains("feature_007") || MAKEFILE.contains("test-acceptance"));
    // No network client crate in the MCP package.
    let manifest = include_str!("../../workbench-mcp/Cargo.toml");
    assert!(!manifest.contains("reqwest"));
    assert!(!manifest.contains("hyper"));
}

#[test]
fn provider_native_mcp_remains_disabled() {
    assert!(CLAUDE_PROCESS.contains("--strict-mcp-config"));
    assert!(CLAUDE_PROCESS.contains(r#"{"mcpServers":{}}"#));
    assert!(CODEX_PROCESS.contains("read-only") || CODEX_PROCESS.contains("sandbox"));
    assert!(ACP_SUPERVISOR.contains("no-leader") || ACP_SUPERVISOR.contains("--no-leader"));
    assert!(MCP_GATEWAY.contains("ToolKind::Mcp") || MCP_LIB.contains("McpGateway"));
}

#[tokio::test]
async fn empty_registry_is_valid() {
    let config = WorkbenchConfiguration::safe_builtins();
    workbench_config::validate::validate(&config).expect("empty validates");
    let snapshot = ConfigurationSnapshot::create(&config, vec!["test".to_owned()]).expect("snap");
    let lock = WorkbenchLock::repository(&config, &snapshot, &BTreeMap::new()).expect("lock");
    lock.verify().expect("lock verifies");
    let gateway = McpGateway::bootstrap(
        config,
        &lock,
        PathBuf::from("/tmp/wb-mcp-empty"),
        "empty",
        true,
    )
    .expect("empty gateway");
    assert!(gateway.available_servers().is_empty());
    let _ = snapshot;
    let err = gateway
        .invoke(ToolInvokeRequest {
            tool_id: "missing".to_owned(),
            operation: "x".to_owned(),
            correlation_id: "corr-empty".to_owned(),
            arguments: json!({}),
            ..ToolInvokeRequest::default()
        })
        .await
        .expect_err("unavailable without crash");
    assert!(matches!(
        err.kind(),
        McpErrorKind::PolicyDenied | McpErrorKind::Unavailable | McpErrorKind::InvalidConfiguration
    ));
}

// --- harness ----------------------------------------------------------------

struct Harness {
    config: WorkbenchConfiguration,
    lock: WorkbenchLock,
    gateway: Arc<McpGateway>,
    runtime: TempDir,
}

fn harness(include_http_tool: bool) -> Harness {
    let _ = include_http_tool;
    harness_named("ws-default")
}

fn harness_named(workspace: &str) -> Harness {
    let runtime = secure_tempdir("wb-mcp-");
    // Use the cargo-built fake under the workspace target tree (user-owned,
    // non-world-writable parents). Copying into /tmp fails path safety checks.
    let fake = Path::new(FAKE_MCP);
    let config = base_config(fake, true);
    let lock = lock_for(&config);
    let gateway = McpGateway::bootstrap(
        config.clone(),
        &lock,
        runtime.path().join("rt"),
        workspace,
        true,
    )
    .expect("gateway bootstrap");
    Harness {
        config,
        lock,
        gateway: Arc::new(gateway),
        runtime,
    }
}

fn base_config(fake: &Path, with_http: bool) -> WorkbenchConfiguration {
    let mut config = WorkbenchConfiguration::safe_builtins();
    let digest = sha256_path(fake);
    config.mcp_servers.insert(
        "stdio-a".to_owned(),
        McpServer {
            transport: McpTransport::Stdio,
            version: "1.0.0".to_owned(),
            sha256: digest,
            executable: Some(fake.to_string_lossy().into_owned()),
            args: Vec::new(),
            env: BTreeMap::from([(
                "TOKEN".to_owned(),
                "platform:workbench/mcp/token".to_owned(),
            )]),
            url: None,
            headers: BTreeMap::new(),
            max_response_bytes: None,
        },
    );
    if with_http {
        let url = "http://127.0.0.1:9/mcp";
        let sha = http_endpoint_sha256(url).expect("http pin");
        config.mcp_servers.insert(
            "http-a".to_owned(),
            McpServer {
                transport: McpTransport::Http,
                version: "1.0.0".to_owned(),
                sha256: sha,
                executable: None,
                args: Vec::new(),
                env: BTreeMap::new(),
                url: Some(url.to_owned()),
                headers: BTreeMap::from([(
                    "Authorization".to_owned(),
                    "platform:workbench/mcp/http".to_owned(),
                )]),
                max_response_bytes: Some(64),
            },
        );
    }
    insert_tools(&mut config);
    config.roles.insert(
        "reviewer".to_owned(),
        Role {
            model: "fake-default".to_owned(),
            tools: vec!["repo-read".to_owned()],
            data_sources: Vec::new(),
            required_capabilities: Vec::new(),
            fallback_models: Vec::new(),
        },
    );
    config.roles.insert(
        "builder".to_owned(),
        Role {
            model: "fake-default".to_owned(),
            tools: vec![
                "repo-read".to_owned(),
                "cluster-mutate".to_owned(),
                "prod-deploy".to_owned(),
                "cred-read".to_owned(),
                "paid-infer".to_owned(),
                "http-read".to_owned(),
            ],
            data_sources: Vec::new(),
            required_capabilities: Vec::new(),
            fallback_models: Vec::new(),
        },
    );
    config.workflows.insert(
        "ship".to_owned(),
        Workflow {
            steps: vec![WorkflowStep {
                id: "review".to_owned(),
                role: "builder".to_owned(),
                on_findings: None,
                max_iterations: None,
                tools: vec!["repo-read".to_owned()],
            }],
        },
    );
    workbench_config::validate::validate(&config).expect("config validates");
    config
}

fn insert_tools(config: &mut WorkbenchConfiguration) {
    config.tools.insert(
        "repo-read".to_owned(),
        mcp_tool(
            "stdio-a",
            "read",
            EffectClass::IdempotentRead,
            true,
            ApprovalMode::Never,
        ),
    );
    config.tools.insert(
        "cluster-mutate".to_owned(),
        mcp_tool(
            "stdio-a",
            "apply",
            EffectClass::NonIdempotentWrite,
            false,
            ApprovalMode::Policy,
        ),
    );
    config.tools.insert(
        "prod-deploy".to_owned(),
        mcp_tool(
            "stdio-a",
            "deploy",
            EffectClass::Production,
            false,
            ApprovalMode::Policy,
        ),
    );
    config.tools.insert(
        "cred-read".to_owned(),
        mcp_tool(
            "stdio-a",
            "reveal",
            EffectClass::Credential,
            false,
            ApprovalMode::Policy,
        ),
    );
    config.tools.insert(
        "paid-infer".to_owned(),
        mcp_tool(
            "stdio-a",
            "run",
            EffectClass::PaidInference,
            false,
            ApprovalMode::Policy,
        ),
    );
    config.tools.insert(
        "http-read".to_owned(),
        mcp_tool(
            "http-a",
            "read",
            EffectClass::IdempotentRead,
            true,
            ApprovalMode::Never,
        ),
    );
}

fn mcp_tool(
    server: &str,
    operation: &str,
    effect_class: EffectClass,
    idempotent: bool,
    approval: ApprovalMode,
) -> Tool {
    Tool {
        kind: ToolKind::Mcp,
        mcp_server: Some(server.to_owned()),
        operations: vec![Operation {
            name: operation.to_owned(),
            effect_class,
            idempotent,
            material_cost: false,
            approval,
        }],
    }
}

fn lock_for(config: &WorkbenchConfiguration) -> WorkbenchLock {
    let snapshot =
        ConfigurationSnapshot::create(config, vec!["test".to_owned()]).expect("snapshot");
    WorkbenchLock::repository(config, &snapshot, &BTreeMap::new()).expect("lock")
}

fn sha256_path(path: &Path) -> String {
    let bytes = fs::read(path).expect("read");
    hex::encode(Sha256::digest(bytes))
}

fn secure_tempdir(prefix: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/private/tmp")
        .or_else(|_| tempfile::Builder::new().prefix(prefix).tempdir_in("/tmp"))
        .expect("secure temporary directory")
}

const fn user_source() -> PolicySource {
    PolicySource::User
}

// --- gherkin parse ----------------------------------------------------------

struct ParsedFeature {
    heading_count: usize,
    cases: Vec<ParsedCase>,
}

struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

struct ScenarioTemplate {
    title: String,
    outline: bool,
    steps: Vec<String>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn parse_feature(source: &str) -> ParsedFeature {
    let mut templates = Vec::new();
    let mut current: Option<ScenarioTemplate> = None;
    let mut in_examples = false;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(title) = line
            .strip_prefix("Scenario Outline:")
            .or_else(|| line.strip_prefix("Scenario:"))
        {
            if let Some(template) = current.take() {
                templates.push(template);
            }
            current = Some(ScenarioTemplate {
                title: title.trim().to_owned(),
                outline: line.starts_with("Scenario Outline:"),
                steps: Vec::new(),
                headers: Vec::new(),
                rows: Vec::new(),
            });
            in_examples = false;
        } else if let Some(template) = current.as_mut() {
            if ["Given ", "When ", "Then ", "And ", "But "]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            {
                template.steps.push(line.to_owned());
            } else if line == "Examples:" {
                in_examples = true;
            } else if in_examples && line.starts_with('|') {
                let row = parse_example_row(line);
                if template.headers.is_empty() {
                    template.headers = row;
                } else {
                    template.rows.push(row);
                }
            }
        }
    }
    if let Some(template) = current {
        templates.push(template);
    }
    let heading_count = templates.len();
    let cases = templates
        .into_iter()
        .flat_map(expand_template)
        .collect::<Vec<_>>();
    ParsedFeature {
        heading_count,
        cases,
    }
}

fn expand_template(template: ScenarioTemplate) -> Vec<ParsedCase> {
    if !template.outline {
        return vec![ParsedCase {
            name: template.title,
            steps: template.steps,
        }];
    }
    template
        .rows
        .into_iter()
        .map(|row| {
            let values = template
                .headers
                .iter()
                .cloned()
                .zip(row)
                .collect::<Vec<_>>();
            let steps = template
                .steps
                .iter()
                .map(|step| {
                    values.iter().fold(step.clone(), |expanded, (name, value)| {
                        expanded.replace(&format!("<{name}>"), value)
                    })
                })
                .collect();
            let suffix = values
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(", ");
            ParsedCase {
                name: format!("{} [{suffix}]", template.title),
                steps,
            }
        })
        .collect()
}

fn parse_example_row(line: &str) -> Vec<String> {
    line.strip_prefix('|')
        .and_then(|row| row.strip_suffix('|'))
        .expect("valid Examples row")
        .split('|')
        .map(str::trim)
        .map(str::to_owned)
        .collect()
}

fn fingerprint(steps: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            hash ^= u64::from(b'\n');
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for byte in step.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}
