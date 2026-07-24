use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::Duration,
};

use futures_util::StreamExt as _;
use serde_json::Value;
use tempfile::TempDir;
use uuid::Uuid;
use workbench_claude::{ClaudeLaunchProfile, ClaudeProviderAdapter};
use workbench_config::{
    AdapterInput, WorkbenchConfiguration,
    model::{ProviderDriver, ProviderType},
};
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{CancellationStatus, ProviderAdapter, ProviderOutput, ProviderPrompt},
    value::{NonEmptyText, ProviderId},
};
use workbench_daemon::{
    Application, FakeBehavior, StartupConfiguration,
    providers::{ProviderRuntime, ProviderRuntimeError},
};
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, EventKind, PROTOCOL_V1,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, EmptyParams,
        ExportParams, PromptParams,
    },
    response::{ApprovalResult, AttachSessionResult, CreateSessionResult, PromptResult},
};
use workbench_storage::{MemoryKeyStore, SqliteStorage};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/add-a-supervised-claude-code-subscription-adapter-that-pins.feature"
);
const CLAUDE_MANIFEST: &str = include_str!("../../workbench-claude/Cargo.toml");
const CLAUDE_ADAPTER_SOURCE: &str = include_str!("../../workbench-claude/src/adapter.rs");
const CLAUDE_PROCESS_SOURCE: &str = include_str!("../../workbench-claude/src/process.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const FAKE_CLAUDE: &str = env!("CARGO_BIN_EXE_fake_claude");
const FAKE_VERSION: &str = "2.1.218";
const RESPONSE_DEADLINE: Duration = Duration::from_secs(3);
const MODE_FILE: &str = ".workbench-fake-claude-mode";
const OBSERVATION_FILE: &str = ".workbench-fake-claude-observation";
const SECRET_MARKERS: [&str; 7] = [
    "AUTH-MARKER-F005",
    "STDERR-MARKER-F005",
    "SESSION-MARKER-F005",
    "THINKING-MARKER-F005",
    "TOOL-MARKER-F005",
    "USAGE-MARKER-F005",
    "ENV-MARKER-F005",
];

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 27] = [
    binding(
        "Execute through the offline fake",
        0xfda6_db9b_7bff_eab4,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Enforce the pinned launch profile",
        0x009e_098e_ed79_4120,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Reject an ineligible authentication mode [auth_state=not logged in]",
        0xee4a_e3db_0740_a7dc,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Reject an ineligible authentication mode [auth_state=API key]",
        0xafe8_57da_46ff_7285,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Reject an ineligible authentication mode [auth_state=alternate provider]",
        0xf8ac_0a6a_3bb6_92f5,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Reject executable replacement",
        0x6438_f2a4_3dae_07cd,
        "lock_and_codec_boundaries_are_enforced",
    ),
    binding(
        "Correlate initialization",
        0x40ef_9500_9984_5d0b,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Enforce frame boundaries [size=exactly 8 MiB, outcome=accepted]",
        0x288c_4d8e_8479_3529,
        "lock_and_codec_boundaries_are_enforced",
    ),
    binding(
        "Enforce frame boundaries [size=one byte over 8 MiB, outcome=rejected]",
        0xff07_5c2b_4a62_9b76,
        "lock_and_codec_boundaries_are_enforced",
    ),
    binding(
        "Reject malformed stream input [malformed_input=duplicate keys]",
        0xe5e1_f92a_1ac1_c632,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Reject malformed stream input [malformed_input=invalid UTF-8]",
        0x2ec7_3f40_b419_48bc,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Reject malformed stream input [malformed_input=truncated JSON]",
        0xc6c8_c6c6_c459_b603,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Reject malformed stream input [malformed_input=an empty frame]",
        0xd6a8_3685_96f4_715a,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Reject malformed stream input [malformed_input=an invalid envelope]",
        0xd009_dcb2_dea2_5889,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Contain native tools",
        0x9d56_2b46_67d4_ad9a,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Normalize partial and final output",
        0xb892_6f91_2fd0_39bc,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Fail before external dispatch",
        0xa4e0_dd69_a4ea_4789,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Preserve uncertainty after an active crash",
        0x086b_7aff_4686_2213,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Confirm cancellation from the terminal result",
        0x7a35_e497_66b2_4f88,
        "cancellation_requires_acknowledgement_and_aborted_result",
    ),
    binding(
        "Preserve uncertainty for unconfirmed cancellation [unconfirmed_outcome=acknowledgment only]",
        0x78a4_2fa8_9de5_d5ff,
        "cancellation_requires_acknowledgement_and_aborted_result",
    ),
    binding(
        "Preserve uncertainty for unconfirmed cancellation [unconfirmed_outcome=error result]",
        0xf074_e651_1e85_e4df,
        "cancellation_requires_acknowledgement_and_aborted_result",
    ),
    binding(
        "Preserve uncertainty for unconfirmed cancellation [unconfirmed_outcome=silence]",
        0xff84_3862_694d_0c65,
        "daemon_cancels_during_provider_setup_within_the_public_deadline",
    ),
    binding(
        "Preserve uncertainty for unconfirmed cancellation [unconfirmed_outcome=end of stream]",
        0x34ac_d358_698d_d252,
        "cancellation_requires_acknowledgement_and_aborted_result",
    ),
    binding(
        "Preserve uncertainty for unconfirmed cancellation [unconfirmed_outcome=process crash]",
        0x192c_2921_38ed_a1d2,
        "cancellation_requires_acknowledgement_and_aborted_result",
    ),
    binding(
        "Contain sensitive process data",
        0xe39b_6eb0_145a_78d0,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Isolate workspace shutdown",
        0x63d9_aff2_72a1_b990,
        "workspace_adapters_are_independently_reaped",
    ),
    binding(
        "Keep the default suite quota free",
        0x63f7_479e_804d_5a1e,
        "default_suite_uses_only_the_committed_fake",
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
fn repository_owned_gherkin_has_twenty_seven_fingerprinted_cases() {
    let parsed = parse_feature(FEATURE);
    assert_eq!(parsed.heading_count, 16);
    assert_eq!(parsed.raw_step_count, 48);
    assert_eq!(parsed.cases.len(), 27);
    assert_eq!(
        parsed
            .cases
            .iter()
            .map(|case| case.steps.len())
            .sum::<usize>(),
        81
    );
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(bindings.len(), 27);
    for case in parsed.cases {
        let binding = bindings
            .get(case.name.as_str())
            .unwrap_or_else(|| panic!("missing binding for {}", case.name));
        assert_eq!(
            fingerprint(&case.steps),
            binding.fingerprint,
            "scenario drifted: {}",
            case.name
        );
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = authentication_and_initialization_fail_closed;
    let _ = cancellation_requires_acknowledgement_and_aborted_result;
    let _ = daemon_cancels_during_provider_setup_within_the_public_deadline;
    let _ = default_suite_uses_only_the_committed_fake;
    let _ = lock_and_codec_boundaries_are_enforced;
    let _ = native_tool_malformed_stream_and_crash_fail_closed;
    let _ = provider_runtime_and_daemon_execute_the_offline_flow;
    let _ = workspace_adapters_are_independently_reaped;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "authentication_and_initialization_fail_closed",
            "cancellation_requires_acknowledgement_and_aborted_result",
            "daemon_cancels_during_provider_setup_within_the_public_deadline",
            "default_suite_uses_only_the_committed_fake",
            "lock_and_codec_boundaries_are_enforced",
            "native_tool_malformed_stream_and_crash_fail_closed",
            "provider_runtime_and_daemon_execute_the_offline_flow",
            "workspace_adapters_are_independently_reaped",
        ])
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn provider_runtime_and_daemon_execute_the_offline_flow() {
    let workspace = secure_tempdir("wb-claude-e2e-");
    set_mode(workspace.path(), "happy");
    let startup = claude_startup(workspace.path(), Path::new(FAKE_CLAUDE));
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("Claude provider runtime");
    let catalog = providers.catalog();
    let capabilities = catalog.get("fake").expect("Claude provider catalog");
    assert_eq!(capabilities.protocol, "claude-code-stream-json/1");
    assert_eq!(capabilities.adapter_version, FAKE_VERSION);
    assert!(
        providers
            .registry()
            .adapter(&ProviderId::parse("fake").expect("provider ID"))
            .is_some()
    );

    let storage_directory = workspace.path().join("storage");
    fs::create_dir(&storage_directory).expect("storage directory");
    fs::set_permissions(&storage_directory, fs::Permissions::from_mode(0o700))
        .expect("storage permissions");
    let database = storage_directory.join("workbench.sqlite");
    let storage =
        SqliteStorage::open(&database, MemoryKeyStore::new()).expect("encrypted test storage");
    let application = Application::new_with_providers(
        storage,
        startup,
        FakeBehavior::default(),
        providers.registry(),
        catalog,
    );
    let daemon = LocalDaemonHarness::start(application.clone()).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(daemon.endpoint(), "feature-005-controller")
        .await
        .expect("controller");
    let created: CreateSessionResult = serde_json::from_value(
        controller
            .call(command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create session"),
    )
    .expect("create result");
    serde_json::from_value::<PromptResult>(
        controller
            .call(command(
                Some(created.session_id),
                ProtocolCommand::SessionPrompt(PromptParams {
                    text: "offline Claude acceptance".to_owned(),
                    explicit_target: None,
                }),
            ))
            .await
            .expect("prompt"),
    )
    .expect("prompt result");

    let mut observer = ProtocolTestClient::connect(daemon.endpoint(), "feature-005-observer")
        .await
        .expect("observer");
    let attached: AttachSessionResult = serde_json::from_value(
        observer
            .call(command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach"),
    )
    .expect("attach result");
    let mut events = Vec::new();
    while events.len() < usize::try_from(attached.last_sequence).expect("bounded sequence") {
        events.push(observer.next_event().await.expect("replayed event"));
    }
    let approval_id = events
        .iter()
        .find(|event| event.kind == EventKind::ApprovalRequested)
        .and_then(|event| event.data.get("approval_id"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("approval ID");
    serde_json::from_value::<ApprovalResult>(
        controller
            .call(command(
                Some(created.session_id),
                ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                    approval_id,
                    decision: ApprovalDecision::Grant,
                }),
            ))
            .await
            .expect("approval"),
    )
    .expect("approval result");

    tokio::time::timeout(RESPONSE_DEADLINE, async {
        while !events
            .iter()
            .any(|event| event.kind == EventKind::SessionCompleted)
        {
            events.push(observer.next_event().await.expect("provider event"));
        }
    })
    .await
    .expect("terminal provider event");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::DispatchStarted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::ProviderEvent)
            .count(),
        1,
        "partial and final text must not be duplicated"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::SessionCompleted)
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|event| { event.kind == EventKind::ToolEvent && event.data["content"] == "Read" })
    );
    let public = serde_json::to_string(&events).expect("serialized public events");
    for marker in SECRET_MARKERS {
        assert!(!public.contains(marker), "secret marker escaped: {marker}");
    }
    let identity = age::x25519::Identity::generate();
    let export_path = storage_directory.join("session.age");
    let export = controller
        .call(command(
            Some(created.session_id),
            ProtocolCommand::SessionExport(ExportParams {
                output_path: export_path.to_string_lossy().into_owned(),
                age_recipients: vec![identity.to_public().to_string()],
            }),
        ))
        .await
        .expect("encrypted export");
    let mut boundaries = serde_json::to_string(&export).expect("export reply");
    let ciphertext = fs::read(&export_path).expect("export ciphertext");
    boundaries.push_str(&String::from_utf8_lossy(&ciphertext));
    let plaintext = age::decrypt(&identity, &ciphertext).expect("decrypt export");
    boundaries.push_str(&String::from_utf8(plaintext).expect("export UTF-8"));

    let observation = observe(workspace.path());
    for required in [
        "--output-format=stream-json",
        "--input-format=stream-json",
        "--permission-mode=dontAsk",
        "--tools=Read,Glob,Grep",
        "--allowedTools=Read,Glob,Grep",
        "--safe-mode",
        "--disable-slash-commands",
        "--no-chrome",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--mcp-config={\"mcpServers\":{}}",
        "--setting-sources=",
        "autoupdater=1",
    ] {
        assert!(
            observation.contains(required),
            "missing launch proof: {required}"
        );
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        assert!(observation.contains(&format!("{name}_present=false")));
    }

    set_mode(workspace.path(), "crash");
    let crashed: CreateSessionResult = serde_json::from_value(
        controller
            .call(command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create crash session"),
    )
    .expect("crash create result");
    serde_json::from_value::<PromptResult>(
        controller
            .call(command(
                Some(crashed.session_id),
                ProtocolCommand::SessionPrompt(PromptParams {
                    text: "crash after dispatch".to_owned(),
                    explicit_target: None,
                }),
            ))
            .await
            .expect("crash prompt"),
    )
    .expect("crash prompt result");
    let mut crash_observer =
        ProtocolTestClient::connect(daemon.endpoint(), "feature-005-crash-observer")
            .await
            .expect("crash observer");
    let crash_attached: AttachSessionResult = serde_json::from_value(
        crash_observer
            .call(command(
                Some(crashed.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("crash attach"),
    )
    .expect("crash attach result");
    let mut crash_events = Vec::new();
    while crash_events.len()
        < usize::try_from(crash_attached.last_sequence).expect("bounded sequence")
    {
        crash_events.push(
            crash_observer
                .next_event()
                .await
                .expect("crash replayed event"),
        );
    }
    let crash_approval = crash_events
        .iter()
        .find(|event| event.kind == EventKind::ApprovalRequested)
        .and_then(|event| event.data.get("approval_id"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("crash approval ID");
    controller
        .call(command(
            Some(crashed.session_id),
            ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                approval_id: crash_approval,
                decision: ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("crash approval");
    tokio::time::timeout(RESPONSE_DEADLINE, async {
        while !crash_events
            .iter()
            .any(|event| event.kind == EventKind::OutcomeUnknown)
        {
            crash_events.push(
                crash_observer
                    .next_event()
                    .await
                    .expect("crash provider event"),
            );
        }
    })
    .await
    .expect("durable crash outcome");
    assert_eq!(
        crash_events
            .iter()
            .filter(|event| event.kind == EventKind::DispatchStarted)
            .count(),
        1,
        "an uncertain crash must not be retried"
    );
    assert!(!crash_events.iter().any(|event| matches!(
        event.kind,
        EventKind::SessionCompleted | EventKind::SessionFailed
    )));
    boundaries.push_str(&serde_json::to_string(&crash_events).expect("crash public events"));
    let crash_export_path = storage_directory.join("crash-session.age");
    let crash_export = controller
        .call(command(
            Some(crashed.session_id),
            ProtocolCommand::SessionExport(ExportParams {
                output_path: crash_export_path.to_string_lossy().into_owned(),
                age_recipients: vec![identity.to_public().to_string()],
            }),
        ))
        .await
        .expect("encrypted crash export");
    boundaries.push_str(&serde_json::to_string(&crash_export).expect("crash export reply"));
    let crash_ciphertext = fs::read(&crash_export_path).expect("crash export ciphertext");
    boundaries.push_str(&String::from_utf8_lossy(&crash_ciphertext));
    boundaries.push_str(
        &String::from_utf8(
            age::decrypt(&identity, &crash_ciphertext).expect("decrypt crash export"),
        )
        .expect("crash export UTF-8"),
    );
    drop(crash_observer);
    drop(observer);
    drop(controller);
    drop(daemon);
    application
        .prepare_shutdown()
        .await
        .expect("application shutdown");
    providers.shutdown().await.expect("provider shutdown");
    drop(providers);
    drop(application);
    append_boundary_files(workspace.path(), &mut boundaries);
    for marker in SECRET_MARKERS {
        assert!(
            !boundaries.contains(marker),
            "stored or exported boundary leaked: {marker}"
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn daemon_cancels_during_provider_setup_within_the_public_deadline() {
    let workspace = secure_tempdir("wb-claude-start-cancel-");
    set_mode(workspace.path(), "prompt-init-hang");
    let startup = claude_startup(workspace.path(), Path::new(FAKE_CLAUDE));
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("Claude provider runtime");
    let storage_directory = workspace.path().join("cancel-storage");
    fs::create_dir(&storage_directory).expect("storage directory");
    fs::set_permissions(&storage_directory, fs::Permissions::from_mode(0o700))
        .expect("storage permissions");
    let storage = SqliteStorage::open(
        &storage_directory.join("cancel.sqlite"),
        MemoryKeyStore::new(),
    )
    .expect("encrypted test storage");
    let application = Application::new_with_providers(
        storage,
        startup,
        FakeBehavior::default(),
        providers.registry(),
        providers.catalog(),
    );
    let daemon = LocalDaemonHarness::start(application.clone()).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(daemon.endpoint(), "setup-cancel-controller")
        .await
        .expect("controller");
    let created: CreateSessionResult = serde_json::from_value(
        controller
            .call(command(
                None,
                ProtocolCommand::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: None,
                }),
            ))
            .await
            .expect("create session"),
    )
    .expect("create result");
    serde_json::from_value::<PromptResult>(
        controller
            .call(command(
                Some(created.session_id),
                ProtocolCommand::SessionPrompt(PromptParams {
                    text: "cancel during provider initialization".to_owned(),
                    explicit_target: None,
                }),
            ))
            .await
            .expect("prompt"),
    )
    .expect("prompt result");
    let mut observer = ProtocolTestClient::connect(daemon.endpoint(), "setup-cancel-observer")
        .await
        .expect("observer");
    let attached: AttachSessionResult = serde_json::from_value(
        observer
            .call(command(
                Some(created.session_id),
                ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach"),
    )
    .expect("attach result");
    let mut events = Vec::new();
    while events.len() < usize::try_from(attached.last_sequence).expect("bounded sequence") {
        events.push(observer.next_event().await.expect("replayed event"));
    }
    let approval_id = events
        .iter()
        .find(|event| event.kind == EventKind::ApprovalRequested)
        .and_then(|event| event.data.get("approval_id"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("approval ID");
    controller
        .call(command(
            Some(created.session_id),
            ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                approval_id,
                decision: ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("approval");

    let started = tokio::time::Instant::now();
    controller
        .call(command(
            Some(created.session_id),
            ProtocolCommand::SessionCancel(EmptyParams {}),
        ))
        .await
        .expect("cancel request");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !events
            .iter()
            .any(|event| event.kind == EventKind::OutcomeUnknown)
        {
            events.push(observer.next_event().await.expect("cancellation event"));
        }
    })
    .await
    .expect("public cancellation deadline");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::DispatchStarted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::OutcomeUnknown)
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| event.kind == EventKind::SessionCompleted)
    );
    assert!(!observe(workspace.path()).contains("prompt"));

    drop(observer);
    drop(controller);
    drop(daemon);
    application
        .prepare_shutdown()
        .await
        .expect("application shutdown");
    providers.shutdown().await.expect("provider shutdown");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn authentication_and_initialization_fail_closed() {
    for mode in [
        "auth-not-logged",
        "auth-api",
        "auth-alternate",
        "init-error",
    ] {
        let workspace = secure_tempdir("wb-claude-preflight-");
        set_mode(workspace.path(), mode);
        let error = ClaudeProviderAdapter::connect(
            provider_id(),
            FAKE_VERSION.to_owned(),
            profile(&workspace),
            Duration::from_millis(150),
        )
        .await
        .err()
        .expect("preflight failure");
        assert!(matches!(
            error.category(),
            FailureCategory::ProviderUnavailable | FailureCategory::CapabilityUnavailable
        ));
        assert!(!observe(workspace.path()).contains("prompt"));
    }

    let workspace = secure_tempdir("wb-claude-correlation-");
    set_mode(workspace.path(), "init-interleaved");
    let adapter = connect(&workspace, Duration::from_millis(150)).await;
    assert!(!observe(workspace.path()).contains("prompt"));
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, provider_prompt("correlated prompt"))
        .await
        .expect("correlated prompt");
    while let Some(output) = stream.next().await {
        output.expect("successful correlated output");
    }
    assert!(observe(workspace.path()).contains("prompt"));
    assert!(adapter.shutdown().await.reaped);
}

#[test]
fn lock_and_codec_boundaries_are_enforced() {
    let workspace = tempfile::Builder::new()
        .prefix("wb-claude-lock-")
        .tempdir_in(std::env::current_dir().expect("current directory"))
        .expect("repository-local secure workspace");
    let executable = workspace.path().join("fake-claude-copy");
    fs::copy(FAKE_CLAUDE, &executable).expect("copy fake executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("fake permissions");
    let startup = claude_startup(workspace.path(), &executable);
    let mut bytes = fs::read(&executable).expect("fake bytes");
    bytes.push(0);
    fs::write(&executable, bytes).expect("replace executable bytes");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let error = runtime
        .block_on(ProviderRuntime::bootstrap(
            &startup,
            workspace.path(),
            workspace.path(),
        ))
        .err()
        .expect("digest mismatch");
    assert!(matches!(error, ProviderRuntimeError::Incompatible(_)));
    assert!(!observe(workspace.path()).contains("initialize"));

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository");
    let output = Command::new(env!("CARGO"))
        .args([
            "test",
            "-p",
            "workbench-claude",
            "codec::tests::",
            "--locked",
            "--",
            "--nocapture",
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .current_dir(repository)
        .output()
        .expect("codec contract tests");
    assert!(
        output.status.success(),
        "codec contract tests failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn native_tool_malformed_stream_and_crash_fail_closed() {
    for mode in [
        "denied-tool",
        "malformed-duplicate",
        "malformed-utf8",
        "malformed-truncated",
        "malformed-empty",
        "malformed-envelope",
        "crash",
    ] {
        let workspace = secure_tempdir("wb-claude-failure-");
        set_mode(workspace.path(), mode);
        let adapter = connect(&workspace, Duration::from_millis(150)).await;
        let handle = adapter.start_session().await.expect("session");
        let mut stream = adapter
            .prompt_stream(&handle, provider_prompt("exercise failure"))
            .await
            .expect("prompt started");
        assert!(matches!(
            stream.next().await,
            Some(Ok(ProviderOutput::Acknowledged { .. }))
        ));
        let failure = tokio::time::timeout(RESPONSE_DEADLINE, stream.next())
            .await
            .unwrap_or_else(|_| panic!("bounded failure for mode {mode}"))
            .expect("terminal failure")
            .expect_err("failure must not complete");
        assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
        assert!(!failure.definite);
        assert!(adapter.shutdown().await.reaped);
    }
}

#[tokio::test]
async fn cancellation_requires_acknowledgement_and_aborted_result() {
    for (mode, expected) in [
        ("wait-cancel", CancellationStatus::Confirmed),
        ("cancel-ack-only", CancellationStatus::Unconfirmed),
        ("cancel-result-before-ack", CancellationStatus::Unconfirmed),
        ("cancel-error-result", CancellationStatus::Unconfirmed),
        ("cancel-silence", CancellationStatus::Unconfirmed),
        ("cancel-eof", CancellationStatus::Unconfirmed),
        ("cancel-crash", CancellationStatus::Unconfirmed),
    ] {
        let workspace = secure_tempdir("wb-claude-cancel-");
        set_mode(workspace.path(), mode);
        let adapter = connect(&workspace, Duration::from_millis(120)).await;
        let handle = adapter.start_session().await.expect("session");
        let prompt = provider_prompt("wait for cancellation");
        let attempt_id = prompt.attempt_id;
        let mut stream = adapter
            .prompt_stream(&handle, prompt)
            .await
            .expect("prompt");
        assert!(stream.next().await.is_some());
        assert_eq!(
            adapter.cancel(&handle, attempt_id).await.expect("cancel"),
            expected
        );
        assert!(stream.next().await.is_none());
        assert!(adapter.shutdown().await.reaped);
        assert_eq!(
            observe(workspace.path())
                .lines()
                .filter(|line| *line == "interrupt")
                .count(),
            1
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn workspace_adapters_are_independently_reaped() {
    let first = secure_tempdir("wb-claude-workspace-a-");
    let second = secure_tempdir("wb-claude-workspace-b-");
    set_mode(first.path(), "cancel-silence");
    set_mode(second.path(), "wait-cancel");
    let first_startup = claude_startup(first.path(), Path::new(FAKE_CLAUDE));
    let second_startup = claude_startup(second.path(), Path::new(FAKE_CLAUDE));
    let first_providers = ProviderRuntime::bootstrap(&first_startup, first.path(), first.path())
        .await
        .expect("first provider runtime");
    let second_providers =
        ProviderRuntime::bootstrap(&second_startup, second.path(), second.path())
            .await
            .expect("second provider runtime");
    let first_storage_directory = first.path().join("storage");
    let second_storage_directory = second.path().join("storage");
    for directory in [&first_storage_directory, &second_storage_directory] {
        fs::create_dir(directory).expect("storage directory");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("storage permissions");
    }
    let first_application = Application::new_with_providers(
        SqliteStorage::open(
            &first_storage_directory.join("workbench.sqlite"),
            MemoryKeyStore::new(),
        )
        .expect("first storage"),
        first_startup,
        FakeBehavior::default(),
        first_providers.registry(),
        first_providers.catalog(),
    );
    let second_application = Application::new_with_providers(
        SqliteStorage::open(
            &second_storage_directory.join("workbench.sqlite"),
            MemoryKeyStore::new(),
        )
        .expect("second storage"),
        second_startup,
        FakeBehavior::default(),
        second_providers.registry(),
        second_providers.catalog(),
    );
    let first_daemon = LocalDaemonHarness::start(first_application.clone()).expect("first daemon");
    let second_daemon =
        LocalDaemonHarness::start(second_application.clone()).expect("second daemon");
    let first_adapter = first_providers
        .registry()
        .adapter(&ProviderId::parse("fake").expect("provider ID"))
        .expect("first adapter");
    let second_adapter = second_providers
        .registry()
        .adapter(&ProviderId::parse("fake").expect("provider ID"))
        .expect("second adapter");
    let first_handle = first_adapter.start_session().await.expect("first session");
    let second_handle = second_adapter
        .start_session()
        .await
        .expect("second session");
    let mut first_stream = first_adapter
        .prompt_stream(&first_handle, provider_prompt("first"))
        .await
        .expect("first prompt");
    let second_prompt = provider_prompt("second");
    let second_attempt = second_prompt.attempt_id;
    let mut second_stream = second_adapter
        .prompt_stream(&second_handle, second_prompt)
        .await
        .expect("second prompt");
    assert!(first_stream.next().await.is_some());
    assert!(second_stream.next().await.is_some());

    drop(first_daemon);
    first_application
        .prepare_shutdown()
        .await
        .expect("first application shutdown");
    first_providers
        .shutdown()
        .await
        .expect("first provider shutdown");
    let mut second_client =
        ProtocolTestClient::connect(second_daemon.endpoint(), "second-workspace-status")
            .await
            .expect("second workspace client");
    second_client
        .call(command(None, ProtocolCommand::StatusGet(EmptyParams {})))
        .await
        .expect("second daemon remains available");
    assert_eq!(
        second_adapter
            .cancel(&second_handle, second_attempt)
            .await
            .expect("second cancel"),
        CancellationStatus::Confirmed
    );
    drop(second_client);
    drop(second_daemon);
    second_application
        .prepare_shutdown()
        .await
        .expect("second application shutdown");
    second_providers
        .shutdown()
        .await
        .expect("second provider shutdown");
    assert!(observe(first.path()).contains("eof"));
    assert!(observe(second.path()).contains("interrupt"));
}

#[test]
fn default_suite_uses_only_the_committed_fake() {
    let fake_name = Path::new(FAKE_CLAUDE)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fake executable name");
    assert!(fake_name.starts_with("fake_claude"));
    assert!(!MAKEFILE.contains("WORKBENCH_CLAUDE_EXECUTABLE"));
    assert!(MAKEFILE.contains("--test feature_005"));
    assert!(
        !MAKEFILE
            .split("test-claude:")
            .nth(1)
            .and_then(|tail| tail.split("\ntest-acceptance:").next())
            .unwrap_or_default()
            .contains("--ignored")
    );
    assert!(!CLAUDE_MANIFEST.contains("tracing"));
    for source in [CLAUDE_ADAPTER_SOURCE, CLAUDE_PROCESS_SOURCE] {
        assert!(!source.contains("tracing::"));
        assert!(!source.contains("println!"));
        assert!(!source.contains("eprintln!"));
    }
}

async fn connect(workspace: &TempDir, deadline: Duration) -> ClaudeProviderAdapter {
    ClaudeProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(workspace),
        deadline,
    )
    .await
    .expect("Claude adapter")
}

fn profile(workspace: &TempDir) -> ClaudeLaunchProfile {
    ClaudeLaunchProfile::new(FAKE_CLAUDE, workspace.path())
        .initialization_timeout(Duration::from_millis(500))
        .shutdown_grace(Duration::from_millis(100))
}

fn provider_id() -> ProviderId {
    ProviderId::parse("claude").expect("provider ID")
}

fn provider_prompt(text: &str) -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: "fable".to_owned(),
        content: NonEmptyText::parse(text).expect("prompt text"),
    }
}

fn claude_startup(repository_root: &Path, executable: &Path) -> StartupConfiguration {
    let executable = executable
        .canonicalize()
        .expect("canonical fake Claude executable");
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    let provider = configuration
        .providers
        .get_mut("fake")
        .expect("built-in provider");
    provider.kind = ProviderType::SubscriptionCli;
    provider.driver = Some(ProviderDriver::ClaudeCode);
    provider.executable = Some(executable.to_string_lossy().into_owned());
    let configuration_directory = repository_root.join(".workbench");
    fs::create_dir_all(&configuration_directory).expect("configuration directory");
    let configuration_path = configuration_directory.join("feature-005-acceptance.yaml");
    fs::write(
        &configuration_path,
        serde_yaml_ng::to_string(&configuration).expect("configuration YAML"),
    )
    .expect("configuration file");
    fs::set_permissions(&configuration_path, fs::Permissions::from_mode(0o600))
        .expect("configuration permissions");
    let inputs = BTreeMap::from([(
        "fake".to_owned(),
        AdapterInput::claude_code(&executable, FAKE_VERSION).expect("Claude adapter input"),
    )]);
    let inspected = StartupConfiguration::inspect_with_adapter_inputs(
        repository_root,
        Some(&configuration_path),
        &inputs,
    )
    .expect("inspect Claude configuration");
    inspected
        .write_base_lock(repository_root)
        .expect("write repository lock");
    StartupConfiguration::load_with_configuration(repository_root, Some(&configuration_path))
        .expect("load verified repository lock")
}

fn command(session_id: Option<Uuid>, command: ProtocolCommand) -> ClientCommand {
    ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id: Uuid::now_v7(),
        session_id,
        command,
    }
}

fn secure_tempdir(prefix: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/private/tmp")
        .or_else(|_| tempfile::Builder::new().prefix(prefix).tempdir_in("/tmp"))
        .expect("secure temporary directory")
}

fn set_mode(workspace: &Path, mode: &str) {
    fs::write(workspace.join(MODE_FILE), mode).expect("fake mode");
}

fn observe(workspace: &Path) -> String {
    fs::read_to_string(workspace.join(OBSERVATION_FILE)).unwrap_or_default()
}

fn append_boundary_files(path: &Path, output: &mut String) {
    for entry in fs::read_dir(path).expect("boundary directory") {
        let entry = entry.expect("boundary entry");
        let child = entry.path();
        if child.file_name().and_then(|name| name.to_str()) == Some(OBSERVATION_FILE) {
            continue;
        }
        if child.is_dir() {
            append_boundary_files(&child, output);
        } else if let Ok(bytes) = fs::read(child) {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
}

struct ParsedFeature {
    heading_count: usize,
    raw_step_count: usize,
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
    let raw_step_count = templates.iter().map(|template| template.steps.len()).sum();
    let cases = templates
        .into_iter()
        .flat_map(expand_template)
        .collect::<Vec<_>>();
    ParsedFeature {
        heading_count,
        raw_step_count,
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
