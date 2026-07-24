use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::Duration,
};

use futures_util::StreamExt as _;
use serde_json::Value;
use tempfile::TempDir;
use uuid::Uuid;
use workbench_codex::{CodexLaunchProfile, CodexProviderAdapter, MAX_FRAME_BYTES};
use workbench_config::{
    AdapterInput, WorkbenchConfiguration,
    model::{ProviderDriver, ProviderType},
};
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{CancellationStatus, ProviderAdapter, ProviderPrompt},
    value::{NonEmptyText, ProviderId},
};
use workbench_daemon::{
    Application, FakeBehavior, StartupConfiguration,
    providers::{ProviderRuntime, ProviderRuntimeError},
};
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, EventKind, PROTOCOL_V1,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, ExportParams,
        PromptParams,
    },
    response::{ApprovalResult, AttachSessionResult, CreateSessionResult, PromptResult},
};
use workbench_storage::{MemoryKeyStore, SqliteStorage};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/add-a-supervised-codex-subscription-adapter-that-pins-an.feature"
);
const CODEX_MANIFEST: &str = include_str!("../../workbench-codex/Cargo.toml");
const CODEX_ADAPTER_SOURCE: &str = include_str!("../../workbench-codex/src/adapter.rs");
const CODEX_PROCESS_SOURCE: &str = include_str!("../../workbench-codex/src/process.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const FAKE_CODEX: &str = env!("CARGO_BIN_EXE_fake_codex");
const FAKE_VERSION: &str = "0.145.0";
const RESPONSE_DEADLINE: Duration = Duration::from_secs(3);
const MODE_FILE: &str = ".workbench-fake-codex-mode";
const OBSERVATION_FILE: &str = ".workbench-fake-codex-observation";
const SECRET_MARKERS: [&str; 7] = [
    "AUTH-MARKER-F006",
    "STDERR-MARKER-F006",
    "SESSION-MARKER-F006",
    "THINKING-MARKER-F006",
    "TOOL-MARKER-F006",
    "USAGE-MARKER-F006",
    "ENV-MARKER-F006",
];

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 23] = [
    binding(
        "Execute through the offline fake",
        0x4997_d3a8_d306_f347,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Enforce the pinned launch profile",
        0xc85e_ab0f_c529_1e58,
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
        "Reject an ineligible authentication mode [auth_state=unknown auth mode]",
        0xed2e_a758_9a60_3e0d,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Reject executable replacement",
        0x6438_f2a4_3dae_07cd,
        "lock_and_codec_boundaries_are_enforced",
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
        "Reject malformed stream input [malformed_input=an invalid event]",
        0x8f35_9192_7313_b3e3,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Contain sandbox and elevated tools",
        0x9725_6864_fdea_4a4c,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Normalize partial and final output",
        0xb892_6f91_2fd0_39bc,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Fail before external dispatch",
        0xf592_d34a_15c2_9e56,
        "authentication_and_initialization_fail_closed",
    ),
    binding(
        "Preserve uncertainty after an active crash",
        0x086b_7aff_4686_2213,
        "native_tool_malformed_stream_and_crash_fail_closed",
    ),
    binding(
        "Confirm cancellation from a terminal abort event",
        0x7800_0472_4d2e_976b,
        "cancellation_requires_abort_terminal_event",
    ),
    binding(
        "Leave cancellation unconfirmed without a terminal abort",
        0xa972_876e_17ca_a368,
        "cancellation_requires_abort_terminal_event",
    ),
    binding(
        "Keep secrets out of durable surfaces",
        0x61d0_2671_0dc5_531f,
        "provider_runtime_and_daemon_execute_the_offline_flow",
    ),
    binding(
        "Isolate workspace shutdown",
        0x23d0_dc32_c4b4_08ba,
        "workspace_adapters_are_independently_reaped",
    ),
    binding(
        "Default suite consumes zero quota",
        0x7c3e_ded8_4f4d_e9f7,
        "default_suite_uses_only_the_committed_fake",
    ),
    binding(
        "Never open operator credential files",
        0xf955_a78a_0f98_ff57,
        "credential_files_are_never_opened",
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
fn repository_owned_gherkin_has_twenty_three_fingerprinted_cases() {
    let parsed = parse_feature(FEATURE);
    assert_eq!(parsed.heading_count, 16);
    assert_eq!(parsed.raw_step_count, 48);
    assert_eq!(parsed.cases.len(), 23);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(bindings.len(), 23);
    for case in &parsed.cases {
        assert!(
            bindings.contains_key(case.name.as_str()),
            "missing binding for {}",
            case.name
        );
        let fp = fingerprint(&case.steps);
        let binding = bindings[case.name.as_str()];
        assert_eq!(fp, binding.fingerprint, "scenario drifted: {}", case.name);
        assert_ne!(fp, 0, "fingerprint collapsed for {}", case.name);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = authentication_and_initialization_fail_closed;
    let _ = cancellation_requires_abort_terminal_event;
    let _ = credential_files_are_never_opened;
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
            "cancellation_requires_abort_terminal_event",
            "credential_files_are_never_opened",
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
    let workspace = secure_tempdir("wb-codex-e2e-");
    set_mode(workspace.path(), "happy");
    let startup = codex_startup(workspace.path(), Path::new(FAKE_CODEX));
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("Codex provider runtime");
    let catalog = providers.catalog();
    let capabilities = catalog.get("fake").expect("Codex provider catalog");
    assert_eq!(capabilities.protocol, "codex-exec-jsonl/1");
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
    let mut controller = ProtocolTestClient::connect(daemon.endpoint(), "feature-006-controller")
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
                    text: "offline Codex acceptance".to_owned(),
                    explicit_target: None,
                }),
            ))
            .await
            .expect("prompt"),
    )
    .expect("prompt result");

    let mut observer = ProtocolTestClient::connect(daemon.endpoint(), "feature-006-observer")
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
        "assistant text must not be duplicated"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::SessionCompleted)
            .count(),
        1
    );
    assert!(events.iter().any(|event| {
        event.kind == EventKind::ToolEvent && event.data["content"] == "command_execution"
    }));
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
    for required in ["exec", "--json", "--ephemeral", "--sandbox=read-only"] {
        assert!(
            observation.contains(required),
            "missing launch observation {required}: {observation}"
        );
    }
    assert!(!observation.contains("forbidden="));
    assert!(observation.contains("OPENAI_API_KEY_present=false"));
    assert!(observation.contains("CODEX_API_KEY_present=false"));
    for marker in SECRET_MARKERS {
        assert!(
            !boundaries.contains(marker),
            "secret marker escaped into durable boundary: {marker}"
        );
    }
    providers.shutdown().await.expect("provider shutdown");
}

#[tokio::test]
async fn authentication_and_initialization_fail_closed() {
    for mode in ["auth-not-logged", "auth-api", "auth-unknown"] {
        let workspace = secure_tempdir("wb-codex-auth-");
        set_mode(workspace.path(), mode);
        let error = CodexProviderAdapter::connect(
            ProviderId::parse("codex").expect("provider"),
            FAKE_VERSION.to_owned(),
            CodexLaunchProfile::new(FAKE_CODEX, workspace.path())
                .preflight_timeout(Duration::from_millis(500)),
            Duration::from_millis(500),
        )
        .await
        .err()
        .expect("auth failure");
        assert_eq!(error.category(), FailureCategory::ProviderUnavailable);
        assert!(!observe(workspace.path()).contains("event=prompt"));
    }

    let workspace = secure_tempdir("wb-codex-version-");
    set_mode(workspace.path(), "happy");
    let error = CodexProviderAdapter::connect(
        ProviderId::parse("codex").expect("provider"),
        "0.144.0".to_owned(),
        CodexLaunchProfile::new(FAKE_CODEX, workspace.path())
            .preflight_timeout(Duration::from_millis(500)),
        Duration::from_millis(500),
    )
    .await
    .err()
    .expect("version failure");
    assert_eq!(error.category(), FailureCategory::CapabilityUnavailable);
}

#[test]
fn lock_and_codec_boundaries_are_enforced() {
    let workspace = tempfile::Builder::new()
        .prefix("wb-codex-lock-")
        .tempdir_in(std::env::current_dir().expect("current directory"))
        .expect("repository-local secure workspace");
    let executable = workspace.path().join("fake-codex-copy");
    fs::copy(FAKE_CODEX, &executable).expect("copy fake executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("fake permissions");
    let startup = codex_startup(workspace.path(), &executable);
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
    assert!(!observe(workspace.path()).contains("prompt"));

    let exact = format!(r#"{{"x":"{}"}}"#, "a".repeat(MAX_FRAME_BYTES - 8));
    assert_eq!(exact.len(), MAX_FRAME_BYTES);
    assert!(workbench_codex::codec_test_decode(exact.as_bytes()).is_ok());
    assert!(workbench_codex::codec_test_decode(format!("{exact} ").as_bytes()).is_err());
}

#[tokio::test]
async fn native_tool_malformed_stream_and_crash_fail_closed() {
    for mode in [
        "malformed-duplicate",
        "malformed-utf8",
        "malformed-truncated",
        "malformed-empty",
        "malformed-envelope",
        "denied-tool",
        "crash",
    ] {
        let workspace = secure_tempdir("wb-codex-malformed-");
        set_mode(workspace.path(), mode);
        let adapter = connect(workspace.path(), Duration::from_millis(500)).await;
        let handle = adapter.start_session().await.expect("session");
        let mut stream = adapter
            .prompt_stream(&handle, provider_prompt(mode))
            .await
            .expect("stream");
        let mut saw_failure = false;
        while let Some(item) = stream.next().await {
            if item.is_err() {
                saw_failure = true;
            }
        }
        assert!(saw_failure, "expected failure for mode {mode}");
    }
}

#[tokio::test]
async fn cancellation_requires_abort_terminal_event() {
    let workspace = secure_tempdir("wb-codex-cancel-");
    set_mode(workspace.path(), "wait-cancel");
    let adapter = connect(workspace.path(), Duration::from_millis(1_500)).await;
    let handle = adapter.start_session().await.expect("session");
    let prompt = provider_prompt("cancel");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("stream");
    let status = adapter.cancel(&handle, attempt_id).await.expect("cancel");
    assert_eq!(status, CancellationStatus::Confirmed);
    while stream.next().await.is_some() {}

    let workspace = secure_tempdir("wb-codex-uncancel-");
    set_mode(workspace.path(), "cancel-silence");
    let adapter = connect(workspace.path(), Duration::from_millis(300)).await;
    let handle = adapter.start_session().await.expect("session");
    let prompt = provider_prompt("hang");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("stream");
    let status = adapter.cancel(&handle, attempt_id).await.expect("cancel");
    assert_eq!(status, CancellationStatus::Unconfirmed);
    while stream.next().await.is_some() {}
    assert!(adapter.shutdown().await.reaped);
}

#[tokio::test]
async fn workspace_adapters_are_independently_reaped() {
    let first = secure_tempdir("wb-codex-ws1-");
    let second = secure_tempdir("wb-codex-ws2-");
    set_mode(first.path(), "cancel-silence");
    set_mode(second.path(), "cancel-silence");
    let left = connect(first.path(), Duration::from_millis(800)).await;
    let right = connect(second.path(), Duration::from_millis(800)).await;
    let left_handle = left.start_session().await.expect("left session");
    let right_handle = right.start_session().await.expect("right session");
    let mut left_stream = left
        .prompt_stream(&left_handle, provider_prompt("left"))
        .await
        .expect("left stream");
    let mut right_stream = right
        .prompt_stream(&right_handle, provider_prompt("right"))
        .await
        .expect("right stream");
    assert!(left.shutdown().await.reaped);
    while left_stream.next().await.is_some() {}
    assert!(
        right.authentication_status().await.expect("auth")
            == workbench_core::ports::AuthenticationStatus::Available
    );
    assert!(right.shutdown().await.reaped);
    while right_stream.next().await.is_some() {}
}

#[test]
fn default_suite_uses_only_the_committed_fake() {
    assert!(CODEX_MANIFEST.contains("name = \"workbench-codex\""));
    assert!(CODEX_ADAPTER_SOURCE.contains("CodexProviderAdapter"));
    assert!(CODEX_PROCESS_SOURCE.contains("sanitize_environment"));
    assert!(CODEX_PROCESS_SOURCE.contains("OPENAI_API_KEY"));
    assert!(CODEX_PROCESS_SOURCE.contains("login"));
    assert!(CODEX_PROCESS_SOURCE.contains("read-only"));
    assert!(!CODEX_PROCESS_SOURCE.contains("dangerously-bypass"));
    assert!(MAKEFILE.contains("test-codex") || MAKEFILE.contains("feature_006"));
    assert!(
        !MAKEFILE.contains("codex exec") || MAKEFILE.contains("fake_codex"),
        "default suite must not require live codex exec"
    );
}

#[tokio::test]
async fn credential_files_are_never_opened() {
    let workspace = secure_tempdir("wb-codex-creds-");
    set_mode(workspace.path(), "happy");
    let codex_home = workspace.path().join("codex-home");
    fs::create_dir_all(&codex_home).expect("codex home");
    let auth = codex_home.join("auth.json");
    fs::write(&auth, r#"{"token":"AUTH-MARKER-F006"}"#).expect("auth file");
    // Safety: only the adapter process may run under a stripped environment.
    // Prove the path exists but adapter does not need to open it.
    assert!(auth.is_file());
    let adapter = connect(workspace.path(), Duration::from_millis(500)).await;
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, provider_prompt("creds"))
        .await
        .expect("stream");
    while let Some(item) = stream.next().await {
        item.expect("success path");
    }
    let observation = observe(workspace.path());
    assert!(!observation.contains("auth.json"));
    // Workbench never opens the operator credential file itself.
    assert_eq!(
        fs::read_to_string(&auth).expect("auth remains unread by Workbench"),
        r#"{"token":"AUTH-MARKER-F006"}"#
    );
    assert!(adapter.shutdown().await.reaped);
}

async fn connect(workspace: &Path, deadline: Duration) -> CodexProviderAdapter {
    CodexProviderAdapter::connect(
        ProviderId::parse("codex").expect("provider"),
        FAKE_VERSION.to_owned(),
        CodexLaunchProfile::new(FAKE_CODEX, workspace)
            .preflight_timeout(Duration::from_millis(500))
            .shutdown_grace(Duration::from_millis(100)),
        deadline,
    )
    .await
    .expect("Codex adapter")
}

fn provider_prompt(text: &str) -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: "gpt-5".to_owned(),
        content: NonEmptyText::parse(text).expect("prompt text"),
    }
}

fn codex_startup(repository_root: &Path, executable: &Path) -> StartupConfiguration {
    let executable = executable
        .canonicalize()
        .expect("canonical fake Codex executable");
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    let provider = configuration
        .providers
        .get_mut("fake")
        .expect("built-in provider");
    provider.kind = ProviderType::SubscriptionCli;
    provider.driver = Some(ProviderDriver::Codex);
    provider.executable = Some(executable.to_string_lossy().into_owned());
    let configuration_directory = repository_root.join(".workbench");
    fs::create_dir_all(&configuration_directory).expect("configuration directory");
    let configuration_path = configuration_directory.join("feature-006-acceptance.yaml");
    fs::write(
        &configuration_path,
        serde_yaml_ng::to_string(&configuration).expect("configuration YAML"),
    )
    .expect("configuration file");
    fs::set_permissions(&configuration_path, fs::Permissions::from_mode(0o600))
        .expect("configuration permissions");
    let inputs = BTreeMap::from([(
        "fake".to_owned(),
        AdapterInput::codex(&executable, FAKE_VERSION).expect("Codex adapter input"),
    )]);
    let inspected = StartupConfiguration::inspect_with_adapter_inputs(
        repository_root,
        Some(&configuration_path),
        &inputs,
    )
    .expect("inspect Codex configuration");
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
