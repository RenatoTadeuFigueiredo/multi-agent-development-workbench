use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, Once, OnceLock,
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use futures_util::StreamExt as _;
use serde_json::{Value, json};
use tempfile::TempDir;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;
use workbench_config::{
    ACP_PROTOCOL, AdapterInput, WorkbenchConfiguration, canonicalize_adapter_executable,
    model::ProviderType, preflight::Authentication,
};
use workbench_core::{
    AttemptId, SessionId,
    ports::{ProviderOutput, ProviderPrompt, Telemetry},
    value::{NonEmptyText, ProviderId},
};
use workbench_daemon::{
    Application, FakeBehavior, StartupConfiguration,
    providers::{ProviderRuntime, probe_adapter_inputs},
};
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, EventKind, PROTOCOL_V1, SessionEvent,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, EmptyParams,
        ExportParams, PromptParams,
    },
    response::{
        AdapterStatus, ApprovalResult, AttachSessionResult, CreateSessionResult, PromptResult,
        SessionResult, SessionState, StatusResult,
    },
};
use workbench_storage::{MemoryKeyStore, SqliteStorage};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/add-a-supervised-acp-adapter-for-an-externally-installed.feature"
);
const MAKEFILE: &str = include_str!("../../../Makefile");
const FAKE_ACP_AGENT: &str = env!("CARGO_BIN_EXE_fake_acp_agent");
const RESPONSE_DEADLINE: Duration = Duration::from_secs(2);
const SUT_DEADLINE: Duration = Duration::from_secs(10);
const MODE_FILE: &str = ".workbench-fake-acp-mode";
const OBSERVATION_FILE: &str = ".workbench-fake-acp-observation.ndjson";
const TRACING_CHILD_ENV: &str = "WORKBENCH_FEATURE_004_TRACING_CHILD";
const SECRET_MARKERS: [&str; 4] = [
    "AUTH-MARKER-F004",
    "SESSION-MARKER-F004",
    "STDERR-MARKER-F004",
    "ERROR-MARKER-F004",
];
const MAX_CAPTURED_LOG_BYTES: usize = 1024 * 1024;

struct ScenarioBinding {
    case_name: &'static str,
    steps: usize,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 23] = [
    ScenarioBinding {
        case_name: "Offline ACP prompt streams through one durable attempt",
        steps: 6,
        fingerprint: 0xae74_fa36_3686_bcf9,
        evidence_test: "provider_runtime_executes_the_offline_acp_flow_end_to_end",
    },
    ScenarioBinding {
        case_name: "The fixed launch profile disables updates",
        steps: 6,
        fingerprint: 0x6207_8ca2_cabe_a37a,
        evidence_test: "provider_runtime_uses_the_fixed_launch_profile",
    },
    ScenarioBinding {
        case_name: "An executable replacement fails before spawn",
        steps: 4,
        fingerprint: 0x3560_18d9_6079_ad4e,
        evidence_test: "provider_runtime_rejects_a_changed_locked_executable",
    },
    ScenarioBinding {
        case_name: "ACP framing is strictly bounded",
        steps: 5,
        fingerprint: 0xfd70_94eb_adfe_f1b6,
        evidence_test: "provider_runtime_enforces_exact_and_oversized_frame_bounds",
    },
    ScenarioBinding {
        case_name: "Malformed ACP input fails closed [invalid_input=duplicate JSON keys]",
        steps: 4,
        fingerprint: 0xc4dc_7bb0_42aa_fe45,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "Malformed ACP input fails closed [invalid_input=invalid UTF-8]",
        steps: 4,
        fingerprint: 0xc8f5_96f4_b089_80d5,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "Malformed ACP input fails closed [invalid_input=truncated JSON]",
        steps: 4,
        fingerprint: 0x921d_d4be_341f_f3f0,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "Malformed ACP input fails closed [invalid_input=invalid JSON-RPC]",
        steps: 4,
        fingerprint: 0xf9ff_1e83_e7a0_c5f1,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "Malformed ACP input fails closed [invalid_input=an empty frame]",
        steps: 4,
        fingerprint: 0x33ff_c1b2_981c_8a7b,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "A pre-dispatch child crash is definite",
        steps: 4,
        fingerprint: 0x6c00_2804_bb32_3236,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "An active child crash becomes uncertain",
        steps: 4,
        fingerprint: 0xf55e_acc5_627e_6121,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Prompt cancellation is explicitly confirmed",
        steps: 4,
        fingerprint: 0x0f6c_327b_9fc7_cf41,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Ambiguous cancellation requires reconciliation [ambiguous_result=acknowledges cancel but leaves prompt running]",
        steps: 5,
        fingerprint: 0xbaf2_27e5_8374_ade4,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Ambiguous cancellation requires reconciliation [ambiguous_result=closes stdout]",
        steps: 5,
        fingerprint: 0xfdfd_db11_4e12_16e1,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Ambiguous cancellation requires reconciliation [ambiguous_result=exits]",
        steps: 5,
        fingerprint: 0x55ef_2b3e_c96b_5f12,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Ambiguous cancellation requires reconciliation [ambiguous_result=returns an error]",
        steps: 5,
        fingerprint: 0x8dcc_8348_b94e_c6f3,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Ambiguous cancellation requires reconciliation [ambiguous_result=completes without stopReason cancelled]",
        steps: 5,
        fingerprint: 0xb052_60ab_2f11_1fda,
        evidence_test: "daemon_maps_crash_and_cancellation_outcomes",
    },
    ScenarioBinding {
        case_name: "Reverse permission is denied",
        steps: 4,
        fingerprint: 0x9450_6646_e785_8ae3,
        evidence_test: "provider_runtime_denies_reverse_permissions",
    },
    ScenarioBinding {
        case_name: "Authentication and diagnostics remain secret",
        steps: 3,
        fingerprint: 0x36fe_2fbf_5cb7_8c75,
        evidence_test: "daemon_contains_provider_owned_secret_markers",
    },
    ScenarioBinding {
        case_name: "A compatible additive update works after re-locking",
        steps: 5,
        fingerprint: 0x00d8_ccca_7fd7_56e5,
        evidence_test: "provider_runtime_accepts_an_explicitly_relocked_additive_update",
    },
    ScenarioBinding {
        case_name: "An incompatible update is unavailable",
        steps: 5,
        fingerprint: 0xef5d_3576_1c16_9e92,
        evidence_test: "provider_runtime_enforces_frame_malformed_and_incompatible_contracts",
    },
    ScenarioBinding {
        case_name: "Workspace shutdown reaps only its child",
        steps: 4,
        fingerprint: 0x8766_7871_464f_f3f0,
        evidence_test: "provider_runtimes_are_workspace_scoped_and_independently_reaped",
    },
    ScenarioBinding {
        case_name: "The default suite consumes no provider quota",
        steps: 4,
        fingerprint: 0x2257_36e6_055a_9b9d,
        evidence_test: "default_suite_uses_only_explicit_offline_fake",
    },
];

struct FakeAcpHarness {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    workspace: TempDir,
    observation: PathBuf,
}

impl FakeAcpHarness {
    fn spawn(mode: &str) -> Self {
        let workspace = tempfile::Builder::new()
            .prefix("wb-fake-acp-")
            .tempdir_in("/tmp")
            .expect("temporary fake ACP workspace");
        let observation = workspace.path().join("observation.ndjson");
        let mut child = Command::new(FAKE_ACP_AGENT)
            .args(["agent", "--no-leader", "stdio"])
            .current_dir(workspace.path())
            .env_clear()
            .env("GROK_DISABLE_AUTOUPDATER", "1")
            .env("WORKBENCH_FAKE_ACP_MODE", mode)
            .env("WORKBENCH_FAKE_ACP_OBSERVATION", &observation)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn explicit fake ACP executable");
        let stdin = child.stdin.take().expect("fake ACP stdin");
        let stdout = child.stdout.take().expect("fake ACP stdout");
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        while line.ends_with('\r') || line.ends_with('\n') {
                            line.pop();
                        }
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self {
            child,
            stdin: Some(stdin),
            lines,
            reader: Some(reader),
            workspace,
            observation,
        }
    }

    fn send(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("fake ACP stdin is open");
        serde_json::to_writer(&mut *stdin, value).expect("serialize fake ACP request");
        stdin.write_all(b"\n").expect("frame delimiter");
        stdin.flush().expect("flush fake ACP request");
    }

    fn receive_raw(&self) -> String {
        self.lines
            .recv_timeout(RESPONSE_DEADLINE)
            .expect("fake ACP response before deadline")
    }

    fn receive(&self) -> Value {
        serde_json::from_str(&self.receive_raw()).expect("valid fake ACP JSON response")
    }

    fn receives_nothing(&self, duration: Duration) {
        assert!(
            self.lines.recv_timeout(duration).is_err(),
            "fake ACP unexpectedly emitted a terminal response"
        );
    }

    fn wait_for_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll fake ACP child") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "fake ACP child did not exit before deadline"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("poll fake ACP child")
            .is_none()
    }

    fn finish(mut self) -> String {
        self.stdin.take();
        self.child.wait().expect("reap fake ACP child");
        if let Some(reader) = self.reader.take() {
            reader.join().expect("join fake ACP stdout reader");
        }
        fs::read_to_string(&self.observation).expect("fake ACP observation log")
    }

    fn workspace_path(&self) -> &std::path::Path {
        self.workspace.path()
    }
}

impl Drop for FakeAcpHarness {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ignored = self.child.kill();
        }
        let _ignored = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ignored = reader.join();
        }
    }
}

#[derive(Debug)]
struct ParsedFeature {
    heading_count: usize,
    raw_step_count: usize,
    cases: Vec<ParsedCase>,
}

#[derive(Debug)]
struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

#[derive(Debug)]
struct ScenarioTemplate {
    title: String,
    outline: bool,
    steps: Vec<String>,
    example_headers: Vec<String>,
    example_rows: Vec<Vec<String>>,
}

#[test]
fn repository_owned_gherkin_has_twenty_three_fingerprinted_cases() {
    assert_repository_bindings();
}

#[test]
fn all_repository_owned_scenario_evidence_executes_in_this_test_binary() {
    assert_repository_bindings();
    let executable = std::env::current_exe().expect("current Feature 004 test binary");
    let evidence_tests = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(evidence_tests.len(), 11);

    for evidence_test in evidence_tests {
        let output = Command::new(&executable)
            .args(["--exact", evidence_test, "--nocapture"])
            .output()
            .unwrap_or_else(|error| panic!("execute repository evidence {evidence_test}: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success()
                && stdout.contains("running 1 test")
                && stdout.contains("1 passed"),
            "repository evidence '{evidence_test}' did not execute successfully\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

fn assert_repository_bindings() {
    let parsed = parse_feature(FEATURE);
    assert_eq!(parsed.heading_count, 15);
    assert_eq!(parsed.raw_step_count, 67);
    assert_eq!(parsed.cases.len(), 23);
    assert_eq!(
        parsed
            .cases
            .iter()
            .map(|case| case.steps.len())
            .sum::<usize>(),
        103
    );
    assert_eq!(SCENARIO_BINDINGS.len(), 23);

    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        bindings.len(),
        SCENARIO_BINDINGS.len(),
        "repository-owned scenario case names must be unique"
    );
    for case in &parsed.cases {
        let binding = bindings
            .get(case.name.as_str())
            .unwrap_or_else(|| panic!("unbound repository-owned scenario case: {}", case.name));
        assert_eq!(
            case.steps.len(),
            binding.steps,
            "step count changed for {}",
            case.name
        );
        assert_eq!(
            fingerprint(&case.steps),
            binding.fingerprint,
            "step or example changed without updating the binding for {}",
            case.name
        );
    }
    assert_eq!(
        parsed
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<BTreeSet<_>>(),
        bindings.keys().copied().collect()
    );
}

fn parse_feature(feature: &str) -> ParsedFeature {
    let mut templates = Vec::new();
    let mut current: Option<ScenarioTemplate> = None;
    let mut in_examples = false;

    for line in feature.lines().map(str::trim) {
        let scenario = line
            .strip_prefix("Scenario: ")
            .map(|title| (title, false))
            .or_else(|| {
                line.strip_prefix("Scenario Outline: ")
                    .map(|title| (title, true))
            });
        if let Some((title, outline)) = scenario {
            if let Some(template) = current.take() {
                templates.push(template);
            }
            current = Some(ScenarioTemplate {
                title: title.to_owned(),
                outline,
                steps: Vec::new(),
                example_headers: Vec::new(),
                example_rows: Vec::new(),
            });
            in_examples = false;
        } else if ["Given ", "When ", "Then ", "And ", "But "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            current
                .as_mut()
                .unwrap_or_else(|| panic!("step appears before a scenario: {line}"))
                .steps
                .push(line.to_owned());
        } else if line == "Examples:" {
            let template = current.as_ref().expect("Examples appears after a scenario");
            assert!(template.outline, "Examples requires a Scenario Outline");
            in_examples = true;
        } else if in_examples && line.starts_with('|') {
            let cells = parse_example_row(line);
            let template = current.as_mut().expect("example row requires a scenario");
            if template.example_headers.is_empty() {
                let unique = cells.iter().collect::<BTreeSet<_>>();
                assert_eq!(unique.len(), cells.len(), "duplicate Examples header");
                template.example_headers = cells;
            } else {
                assert_eq!(
                    cells.len(),
                    template.example_headers.len(),
                    "Examples row width differs from its header"
                );
                template.example_rows.push(cells);
            }
        }
    }
    if let Some(template) = current {
        templates.push(template);
    }

    let heading_count = templates.len();
    let raw_step_count = templates.iter().map(|template| template.steps.len()).sum();
    let cases = expand_templates(templates);
    ParsedFeature {
        heading_count,
        raw_step_count,
        cases,
    }
}

fn expand_templates(templates: Vec<ScenarioTemplate>) -> Vec<ParsedCase> {
    let mut cases = Vec::new();
    for template in templates {
        if !template.outline {
            assert!(
                template.example_headers.is_empty() && template.example_rows.is_empty(),
                "non-outline scenario has Examples: {}",
                template.title
            );
            cases.push(ParsedCase {
                name: template.title,
                steps: template.steps,
            });
            continue;
        }
        assert!(
            !template.example_headers.is_empty() && !template.example_rows.is_empty(),
            "outline has no Examples: {}",
            template.title
        );
        for row in template.example_rows {
            let examples = template.example_headers.iter().zip(row).collect::<Vec<_>>();
            let suffix = examples
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(",");
            let steps = template
                .steps
                .iter()
                .map(|step| {
                    let mut expanded = step.clone();
                    for (name, value) in &examples {
                        expanded = expanded.replace(&format!("<{name}>"), value);
                    }
                    assert!(
                        !expanded.contains('<') && !expanded.contains('>'),
                        "unresolved outline placeholder: {expanded}"
                    );
                    expanded
                })
                .collect();
            cases.push(ParsedCase {
                name: format!("{} [{suffix}]", template.title),
                steps,
            });
        }
    }
    cases
}

fn parse_example_row(line: &str) -> Vec<String> {
    line.strip_prefix('|')
        .and_then(|row| row.strip_suffix('|'))
        .expect("Examples rows must start and end with '|'")
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

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn provider_runtime_executes_the_offline_acp_flow_end_to_end() {
    let workspace = secure_tempdir("wb-acp-e2e-");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("provider runtime bootstrap");
    let catalog = providers.catalog();
    let capabilities = catalog.get("fake").expect("ACP preflight catalog");
    assert_eq!(capabilities.protocol, ACP_PROTOCOL);
    assert_eq!(capabilities.adapter_version, "1.0.0-test");
    assert_eq!(
        capabilities.authentication,
        workbench_config::preflight::Authentication::Available
    );
    assert!(
        providers
            .registry()
            .adapter(&workbench_core::value::ProviderId::parse("fake").expect("provider ID"))
            .is_some()
    );

    let storage =
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("encrypted test storage");
    let application = Application::new_with_providers(
        storage,
        startup,
        FakeBehavior::default(),
        providers.registry(),
        catalog,
    );
    let daemon = LocalDaemonHarness::start(application.clone()).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(daemon.endpoint(), "feature-004-controller")
        .await
        .expect("controller");
    let created = controller
        .call(client_command(
            None,
            ProtocolCommand::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
                workflow: None,
            }),
        ))
        .await
        .expect("create session");
    let created: CreateSessionResult =
        serde_json::from_value(created).expect("session creation result");

    let prompted = controller
        .call(client_command(
            Some(created.session_id),
            ProtocolCommand::SessionPrompt(PromptParams {
                text: "offline provider runtime acceptance".to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("record prompt");
    serde_json::from_value::<PromptResult>(prompted).expect("prompt result");

    let mut observer = ProtocolTestClient::connect(daemon.endpoint(), "feature-004-observer")
        .await
        .expect("observer");
    let attached = observer
        .call(client_command(
            Some(created.session_id),
            ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
        ))
        .await
        .expect("attach session");
    let attached: AttachSessionResult = serde_json::from_value(attached).expect("attach result");
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
    let approved = controller
        .call(client_command(
            Some(created.session_id),
            ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                approval_id,
                decision: ApprovalDecision::Grant,
            }),
        ))
        .await
        .expect("approve provider prompt");
    serde_json::from_value::<ApprovalResult>(approved).expect("approval result");

    tokio::time::timeout(RESPONSE_DEADLINE, async {
        while !events
            .iter()
            .any(|event| event.kind == EventKind::SessionCompleted)
        {
            events.push(observer.next_event().await.expect("live provider event"));
        }
    })
    .await
    .expect("provider terminal event");

    let started = events
        .iter()
        .filter(|event| event.kind == EventKind::DispatchStarted)
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 1);
    let attempt_id = started[0].data["attempt_id"]
        .as_str()
        .expect("started attempt ID");
    for kind in [EventKind::DispatchAcknowledged, EventKind::ProviderEvent] {
        let normalized = events
            .iter()
            .filter(|event| event.kind == kind)
            .collect::<Vec<_>>();
        assert_eq!(normalized.len(), 1, "unexpected {kind:?} count");
        assert_eq!(normalized[0].data["attempt_id"].as_str(), Some(attempt_id));
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::SessionCompleted)
            .count(),
        1
    );
    assert!(events.iter().any(|event| {
        event.kind == EventKind::ProviderEvent
            && event.data["content"] == "deterministic ACP output"
    }));

    drop(observer);
    drop(controller);
    drop(daemon);
    application
        .prepare_shutdown()
        .await
        .expect("durable application shutdown");
    providers
        .shutdown()
        .await
        .expect("provider child shutdown and reap");
}

#[tokio::test]
async fn provider_runtime_uses_the_fixed_launch_profile() {
    let workspace = secure_tempdir("wb-acp-launch-profile-");
    set_sut_mode(workspace.path(), "happy");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("provider runtime launch");
    let observation = observation_events(workspace.path());
    let started = observation
        .iter()
        .find(|event| event["event"] == "started")
        .expect("started observation");
    assert_eq!(started["argv"], json!(["agent", "--no-leader", "stdio"]));
    assert_eq!(started["autoupdater"], "1");
    assert_eq!(
        started["cwd"],
        workspace
            .path()
            .canonicalize()
            .expect("canonical workspace")
            .to_string_lossy()
            .as_ref()
    );
    runtime.shutdown().await.expect("launch-profile child reap");
}

#[tokio::test]
async fn provider_runtime_rejects_a_changed_locked_executable() {
    let workspace = secure_tempdir("wb-acp-digest-");
    let executable_directory = tempfile::Builder::new()
        .prefix("wb-acp-locked-bin-")
        .tempdir_in(
            PathBuf::from(FAKE_ACP_AGENT)
                .parent()
                .expect("fake ACP executable parent"),
        )
        .expect("temporary executable directory");
    let executable = executable_directory.path().join("fake-acp-agent");
    fs::copy(FAKE_ACP_AGENT, &executable).expect("copy fake executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("fake executable permissions");
    let startup = acp_startup(workspace.path(), &executable);
    fs::OpenOptions::new()
        .append(true)
        .open(&executable)
        .and_then(|mut file| file.write_all(b"\nchanged after lock"))
        .expect("replace locked executable");
    set_sut_mode(workspace.path(), "happy");

    let error = match ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path()).await
    {
        Ok(runtime) => {
            runtime
                .shutdown()
                .await
                .expect("unexpected provider runtime shutdown");
            panic!("changed executable must fail before spawn");
        }
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "ACP executable differs from the lock");
    assert!(
        sut_observation(workspace.path()).is_empty(),
        "a changed executable was spawned before its digest failed"
    );

    let regenerated = acp_startup(workspace.path(), &executable);
    let runtime = ProviderRuntime::bootstrap(&regenerated, workspace.path(), workspace.path())
        .await
        .expect("explicit lock regeneration accepts the changed executable");
    assert!(sut_observation(workspace.path()).contains("\"event\":\"started\""));
    runtime.shutdown().await.expect("re-locked child reap");
}

#[tokio::test]
async fn provider_runtime_enforces_frame_malformed_and_incompatible_contracts() {
    for mode in [
        "malformed",
        "duplicate-keys",
        "invalid-utf8",
        "truncated",
        "invalid-jsonrpc",
        "empty-frame",
        "incompatible-version",
        "missing-capability",
    ] {
        let workspace = secure_tempdir(&format!("wb-acp-invalid-{mode}-"));
        set_sut_mode(workspace.path(), mode);
        let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
        let result = tokio::time::timeout(
            SUT_DEADLINE,
            ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path()),
        )
        .await
        .unwrap_or_else(|_| panic!("{mode} exceeded the bounded preflight deadline"));
        let error = match result {
            Ok(runtime) => {
                runtime.shutdown().await.expect("unexpected child reap");
                panic!("{mode} unexpectedly passed provider preflight");
            }
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "provider preflight failed", "{mode}");
        let observation = sut_observation(workspace.path());
        assert!(observation.contains("\"method\":\"initialize\""), "{mode}");
        assert!(
            !observation.contains("\"method\":\"session/prompt\""),
            "{mode}"
        );
        for forbidden in ["duplicate", "ambiguous cancellation error"] {
            assert!(!error.to_string().contains(forbidden), "{mode}");
        }
    }

    let workspace = secure_tempdir("wb-acp-unavailable-health-");
    set_sut_mode(workspace.path(), "happy");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("healthy provider used to inspect public unavailable health");
    let mut unavailable_catalog = runtime.catalog();
    unavailable_catalog
        .get_mut("fake")
        .expect("fake provider catalog")
        .authentication = Authentication::Unavailable;
    let storage =
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("in-memory health storage");
    let application = Application::new_with_providers(
        storage,
        startup,
        FakeBehavior::default(),
        runtime.registry(),
        unavailable_catalog,
    );
    let daemon = LocalDaemonHarness::start(application.clone()).expect("health daemon");
    let mut client = ProtocolTestClient::connect(daemon.endpoint(), "feature-004-health")
        .await
        .expect("health client");
    let status = timed_call(
        &mut client,
        client_command(None, ProtocolCommand::StatusGet(EmptyParams {})),
    )
    .await;
    let status: StatusResult = serde_json::from_value(status).expect("public status");
    let fake = status
        .adapters
        .iter()
        .find(|adapter| adapter.id == "fake")
        .expect("fake adapter health");
    assert_eq!(fake.status, AdapterStatus::Unavailable);
    drop(client);
    drop(daemon);
    application
        .prepare_shutdown()
        .await
        .expect("health application shutdown");
    runtime.shutdown().await.expect("health provider reap");
}

#[tokio::test]
async fn provider_runtime_enforces_exact_and_oversized_frame_bounds() {
    let workspace = secure_tempdir("wb-acp-exact-limit-");
    set_sut_mode(workspace.path(), "exact-limit");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("an exact 8 MiB ACP frame must pass provider preflight");
    assert_eq!(runtime.catalog()["fake"].protocol, ACP_PROTOCOL);
    runtime.shutdown().await.expect("exact-limit child reap");

    let workspace = secure_tempdir("wb-acp-oversize-");
    set_sut_mode(workspace.path(), "oversize");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let error = match ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path()).await
    {
        Ok(runtime) => {
            runtime.shutdown().await.expect("unexpected child reap");
            panic!("an ACP frame larger than 8 MiB must fail provider preflight");
        }
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "provider preflight failed");
}

#[tokio::test]
async fn provider_runtime_denies_reverse_permissions() {
    let workspace = secure_tempdir("wb-acp-reverse-permission-");
    set_sut_mode(workspace.path(), "reverse-permission");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("reverse-permission provider runtime");
    let adapter = runtime
        .registry()
        .adapter(&ProviderId::parse("fake").expect("provider ID"))
        .expect("registered ACP adapter");
    let handle = adapter.start_session().await.expect("provider session");
    let mut stream = adapter
        .prompt_stream(
            &handle,
            ProviderPrompt {
                session_id: SessionId::new(),
                attempt_id: AttemptId::new(),
                runtime_model: "grok-test".to_owned(),
                content: NonEmptyText::parse("deny reverse permission").expect("prompt"),
            },
        )
        .await
        .expect("reverse-permission prompt");
    let mut completed = false;
    while let Some(output) = stream.next().await {
        if matches!(
            output.expect("normalized reverse-permission output"),
            ProviderOutput::Completed { .. }
        ) {
            completed = true;
        }
    }
    assert!(completed);
    let observation = sut_observation(workspace.path());
    assert!(observation.contains("\"event\":\"permission_response\""));
    assert!(observation.contains("\"denied\":true"));
    runtime
        .shutdown()
        .await
        .expect("reverse-permission child reap");
}

#[tokio::test]
async fn provider_runtime_accepts_an_explicitly_relocked_additive_update() {
    let workspace = secure_tempdir("wb-acp-compatible-relock-");
    set_sut_mode(workspace.path(), "compatible-update");
    let executable = canonicalize_adapter_executable(Path::new(FAKE_ACP_AGENT))
        .expect("canonical fake executable");
    let inputs = probe_adapter_inputs(
        &BTreeMap::from([("fake".to_owned(), executable.clone())]),
        workspace.path(),
    )
    .await
    .expect("compatible version probe");
    assert_eq!(inputs["fake"].version, "1.1.0-test");
    let startup = acp_startup_with_inputs(workspace.path(), &executable, &inputs);
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("explicitly re-locked provider");
    assert_eq!(runtime.catalog()["fake"].adapter_version, "1.1.0-test");
    let adapter = runtime
        .registry()
        .adapter(&ProviderId::parse("fake").expect("provider ID"))
        .expect("registered compatible adapter");
    let handle = adapter.start_session().await.expect("provider session");
    let mut stream = adapter
        .prompt_stream(
            &handle,
            ProviderPrompt {
                session_id: SessionId::new(),
                attempt_id: AttemptId::new(),
                runtime_model: "grok-test".to_owned(),
                content: NonEmptyText::parse("compatible additive prompt").expect("prompt"),
            },
        )
        .await
        .expect("compatible prompt");
    let mut acknowledged = 0;
    let mut content = 0;
    let mut completed = 0;
    while let Some(item) = stream.next().await {
        match item.expect("compatible normalized output") {
            ProviderOutput::Acknowledged { .. } => acknowledged += 1,
            ProviderOutput::Content { .. } => content += 1,
            ProviderOutput::Completed { .. } => completed += 1,
            ProviderOutput::Tool { .. } => {}
        }
    }
    assert_eq!((acknowledged, content, completed), (1, 1, 1));
    runtime.shutdown().await.expect("compatible child reap");
}

#[tokio::test]
async fn provider_runtimes_are_workspace_scoped_and_independently_reaped() {
    let first_workspace = secure_tempdir("wb-acp-isolation-a-");
    let second_workspace = secure_tempdir("wb-acp-isolation-b-");
    set_sut_mode(first_workspace.path(), "hang");
    set_sut_mode(second_workspace.path(), "hang");
    let first_startup = acp_startup(first_workspace.path(), Path::new(FAKE_ACP_AGENT));
    let second_startup = acp_startup(second_workspace.path(), Path::new(FAKE_ACP_AGENT));
    let first = ProviderRuntime::bootstrap(
        &first_startup,
        first_workspace.path(),
        first_workspace.path(),
    )
    .await
    .expect("first runtime");
    let second = ProviderRuntime::bootstrap(
        &second_startup,
        second_workspace.path(),
        second_workspace.path(),
    )
    .await
    .expect("second runtime");
    let provider_id = ProviderId::parse("fake").expect("provider ID");
    let first_adapter = first
        .registry()
        .adapter(&provider_id)
        .expect("first adapter");
    let second_adapter = second
        .registry()
        .adapter(&provider_id)
        .expect("second adapter");
    let first_application = Application::new_with_providers(
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("first storage"),
        first_startup,
        FakeBehavior::default(),
        first.registry(),
        first.catalog(),
    );
    let second_application = Application::new_with_providers(
        SqliteStorage::open_in_memory(MemoryKeyStore::new()).expect("second storage"),
        second_startup,
        FakeBehavior::default(),
        second.registry(),
        second.catalog(),
    );

    first_application
        .prepare_shutdown()
        .await
        .expect("first application shutdown");
    first.shutdown().await.expect("first child reap");
    assert_eq!(
        first_adapter
            .authentication_status()
            .await
            .expect("first health"),
        workbench_core::ports::AuthenticationStatus::Unavailable
    );
    assert_eq!(
        second_adapter
            .authentication_status()
            .await
            .expect("second health"),
        workbench_core::ports::AuthenticationStatus::Available
    );
    assert!(sut_observation(first_workspace.path()).contains("\"event\":\"started\""));
    assert!(sut_observation(second_workspace.path()).contains("\"event\":\"started\""));
    second_application
        .prepare_shutdown()
        .await
        .expect("second application shutdown");
    second.shutdown().await.expect("second child reap");
}

#[tokio::test]
async fn daemon_maps_crash_and_cancellation_outcomes() {
    let preflight_workspace = secure_tempdir("wb-acp-preflight-crash-");
    set_sut_mode(preflight_workspace.path(), "crash-initialize");
    let startup = acp_startup(preflight_workspace.path(), Path::new(FAKE_ACP_AGENT));
    assert!(
        ProviderRuntime::bootstrap(
            &startup,
            preflight_workspace.path(),
            preflight_workspace.path(),
        )
        .await
        .is_err()
    );
    let observation = sut_observation(preflight_workspace.path());
    assert!(observation.contains("\"method\":\"initialize\""));
    assert!(!observation.contains("\"method\":\"session/prompt\""));

    let crashed = execute_sut_session("crash-prompt", false, false).await;
    assert_eq!(crashed.session.state, SessionState::OutcomeUnknown);
    assert!(crashed.session.uncertain_attempt_id.is_some());
    assert_eq!(event_count(&crashed.events, EventKind::DispatchStarted), 1);
    assert_eq!(event_count(&crashed.events, EventKind::OutcomeUnknown), 1);
    assert_eq!(
        crashed
            .observation
            .matches("\"method\":\"session/prompt\"")
            .count(),
        1
    );

    let confirmed = execute_sut_session("cancel-confirmed", true, false).await;
    assert_eq!(confirmed.session.state, SessionState::Cancelled);
    assert!(confirmed.cancellation_elapsed.expect("cancellation timing") <= Duration::from_secs(5));
    assert_eq!(
        event_count(&confirmed.events, EventKind::SessionCancelled),
        1
    );
    assert_eq!(event_count(&confirmed.events, EventKind::OutcomeUnknown), 0);
    assert_eq!(
        confirmed
            .observation
            .matches("\"event\":\"cancel_received\"")
            .count(),
        1
    );

    for mode in [
        "cancel-unconfirmed",
        "cancel-eof",
        "cancel-exit",
        "cancel-error",
        "cancel-end-turn",
    ] {
        let ambiguous = execute_sut_session(mode, true, false).await;
        assert_eq!(
            ambiguous.session.state,
            SessionState::OutcomeUnknown,
            "{mode}"
        );
        assert!(ambiguous.session.uncertain_attempt_id.is_some(), "{mode}");
        let cancellation_elapsed = ambiguous.cancellation_elapsed.expect("cancellation timing");
        assert!(
            cancellation_elapsed <= Duration::from_secs(5),
            "{mode} took {cancellation_elapsed:?}"
        );
        assert_eq!(
            event_count(&ambiguous.events, EventKind::CancelConfirmed),
            0,
            "{mode}"
        );
        assert_eq!(
            event_count(&ambiguous.events, EventKind::OutcomeUnknown),
            1,
            "{mode}"
        );
        assert_eq!(
            ambiguous
                .observation
                .matches("\"event\":\"cancel_received\"")
                .count(),
            1,
            "{mode}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn daemon_contains_provider_owned_secret_markers() {
    if std::env::var_os(TRACING_CHILD_ENV).is_none() {
        run_dedicated_tracing_proof();
        return;
    }
    assert_eq!(
        std::env::var(TRACING_CHILD_ENV).as_deref(),
        Ok("1"),
        "invalid Feature 004 tracing child sentinel"
    );
    let captured_logs = global_workbench_logs();
    let log_checkpoint = captured_logs.checkpoint();
    let outcome = execute_sut_session("secret-error", false, true).await;
    let logs = captured_logs.snapshot_since(log_checkpoint);
    assert!(
        logs.contains("protocol command received"),
        "the acceptance test must inspect actual Workbench tracing"
    );
    assert!(
        logs.contains("session.export"),
        "the captured tracing must include the secret fixture's export command"
    );
    assert_eq!(outcome.session.state, SessionState::OutcomeUnknown);
    for marker in SECRET_MARKERS {
        assert!(
            outcome.observation.contains(marker),
            "fake did not emit proof marker {marker}"
        );
        assert!(
            !outcome.workbench_boundaries.contains(marker),
            "Workbench boundary leaked {marker}"
        );
        assert!(
            !outcome.telemetry_labels.contains(marker),
            "telemetry leaked {marker}"
        );
        assert!(!logs.contains(marker), "Workbench tracing leaked {marker}");
    }
}

fn run_dedicated_tracing_proof() {
    let output = Command::new(std::env::current_exe().expect("current Feature 004 test binary"))
        .args([
            "--exact",
            "daemon_contains_provider_owned_secret_markers",
            "--nocapture",
        ])
        .env(TRACING_CHILD_ENV, "1")
        .output()
        .expect("dedicated Feature 004 tracing subprocess");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("running 1 test") && stdout.contains("1 passed"),
        "dedicated tracing proof failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[tokio::test]
#[ignore = "requires an explicitly selected, installed, and authenticated Grok Build"]
async fn live_provider_runtime_initializes_the_digest_pinned_snapshot_without_a_prompt() {
    let executable = PathBuf::from(
        std::env::var_os("WORKBENCH_GROK_EXECUTABLE")
            .expect("WORKBENCH_GROK_EXECUTABLE must select the exact executable"),
    );
    let executable =
        canonicalize_adapter_executable(&executable).expect("safe canonical executable");
    let workspace = secure_tempdir("wb-acp-live-");
    let inputs = probe_adapter_inputs(
        &BTreeMap::from([("fake".to_owned(), executable.clone())]),
        workspace.path(),
    )
    .await
    .expect("bounded live version probe");
    let startup = acp_startup_with_inputs(workspace.path(), &executable, &inputs);
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("digest-pinned live provider runtime");

    assert_eq!(
        providers.catalog()["fake"].authentication,
        workbench_config::preflight::Authentication::Available
    );
    providers
        .shutdown()
        .await
        .expect("live provider child shutdown and reap");
}

#[tokio::test]
async fn default_suite_uses_only_explicit_offline_fake() {
    let workspace = secure_tempdir("wb-acp-default-offline-");
    set_sut_mode(workspace.path(), "happy");
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let runtime = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .expect("default suite explicit fake provider");
    runtime.shutdown().await.expect("default fake child reap");

    let fake_name = Path::new(FAKE_ACP_AGENT)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fake executable file name");
    assert!(fake_name.starts_with("fake_acp_agent"));
    assert!(sut_observation(workspace.path()).contains("\"event\":\"started\""));
    assert!(!MAKEFILE.contains("WORKBENCH_GROK_EXECUTABLE"));
    let target = MAKEFILE
        .split("test-acp:")
        .nth(1)
        .and_then(|tail| tail.split("\ntest-acceptance:").next())
        .expect("test-acp Make target");
    assert!(target.contains("--test feature_004"));
    assert!(!target.contains("--ignored"));
}

#[test]
fn fake_agent_exercises_the_acp_v1_happy_profile_offline() {
    let mut agent = FakeAcpHarness::spawn("happy");
    let workspace = agent.workspace_path().to_path_buf();

    agent.send(&request(
        "initialize-1",
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {
                "name": "workbench-test",
                "title": "Workbench Test",
                "version": "1"
            }
        }),
    ));
    let initialized = agent.receive();
    assert_eq!(initialized["id"], "initialize-1");
    assert_eq!(initialized["result"]["protocolVersion"], 1);
    assert_eq!(
        initialized["result"]["agentCapabilities"]["loadSession"],
        true
    );
    assert_eq!(initialized["result"]["authMethods"], json!([]));

    agent.send(&request(
        "new-1",
        "session/new",
        json!({
            "cwd": workspace,
            "mcpServers": [],
            "_meta": {"modelId": "grok-test"}
        }),
    ));
    assert_eq!(agent.receive()["result"]["sessionId"], "fake-acp-session");

    agent.send(&request(
        "load-1",
        "session/load",
        json!({
            "sessionId": "fake-acp-session",
            "cwd": agent.workspace_path(),
            "mcpServers": []
        }),
    ));
    assert_eq!(agent.receive()["result"]["sessionId"], "fake-acp-session");

    agent.send(&request(
        "prompt-1",
        "session/prompt",
        json!({
            "sessionId": "fake-acp-session",
            "prompt": [{"type": "text", "text": "secret prompt not logged"}]
        }),
    ));
    let update = agent.receive();
    assert_eq!(update["method"], "session/update");
    assert_eq!(
        update["params"]["update"]["sessionUpdate"],
        "agent_message_chunk"
    );
    assert_eq!(agent.receive()["result"]["stopReason"], "end_turn");

    let observation = agent.finish();
    assert!(observation.contains("\"argv\":[\"agent\",\"--no-leader\",\"stdio\"]"));
    assert!(observation.contains("\"autoupdater\":\"1\""));
    assert!(observation.contains(workspace.to_string_lossy().as_ref()));
    assert!(!observation.contains("secret prompt not logged"));
    assert!(!observation.contains("grok agent"));
}

#[test]
fn fake_agent_supports_authentication_and_compatible_additions() {
    let mut authenticated = FakeAcpHarness::spawn("authenticate");
    authenticated.send(&initialize_request("initialize-auth"));
    let initialized = authenticated.receive();
    assert_eq!(
        initialized["result"]["authMethods"][0]["id"],
        "fake-subscription"
    );
    authenticated.send(&request(
        "auth-1",
        "authenticate",
        json!({"methodId": "fake-subscription"}),
    ));
    assert_eq!(authenticated.receive()["result"], json!({}));
    authenticated.finish();

    let mut compatible = FakeAcpHarness::spawn("compatible-update");
    compatible.send(&initialize_request("initialize-compatible"));
    let initialized = compatible.receive();
    assert_eq!(initialized["result"]["protocolVersion"], 1);
    assert_eq!(initialized["result"]["_meta"]["compatibleFixture"], true);
    compatible.send(&prompt_request("prompt-compatible"));
    assert_eq!(compatible.receive()["method"], "session/update");
    assert_eq!(compatible.receive()["method"], "fake/additive_notification");
    assert_eq!(compatible.receive()["result"]["stopReason"], "end_turn");
    compatible.finish();
}

#[test]
fn fake_agent_models_confirmed_and_unconfirmed_cancellation() {
    let mut confirmed = FakeAcpHarness::spawn("cancel-confirmed");
    confirmed.send(&prompt_request("prompt-cancelled"));
    assert_eq!(confirmed.receive()["method"], "session/update");
    confirmed.send(&notification(
        "session/cancel",
        json!({"sessionId": "fake-acp-session"}),
    ));
    let terminal = confirmed.receive();
    assert_eq!(terminal["id"], "prompt-cancelled");
    assert_eq!(terminal["result"]["stopReason"], "cancelled");
    let observation = confirmed.finish();
    assert!(observation.contains("\"event\":\"cancel_received\""));
    assert!(observation.contains("\"stop_reason\":\"cancelled\""));

    for mode in ["cancel-unconfirmed", "hang"] {
        let mut unconfirmed = FakeAcpHarness::spawn(mode);
        unconfirmed.send(&prompt_request("prompt-uncertain"));
        assert_eq!(unconfirmed.receive()["method"], "session/update");
        unconfirmed.send(&notification(
            "session/cancel",
            json!({"sessionId": "fake-acp-session"}),
        ));
        unconfirmed.receives_nothing(Duration::from_millis(100));
        let observation = unconfirmed.finish();
        assert!(observation.contains("\"event\":\"cancel_received\""));
        assert!(!observation.contains("\"stop_reason\":\"cancelled\""));
    }
}

#[test]
fn fake_agent_models_reverse_permission_denial() {
    let mut agent = FakeAcpHarness::spawn("reverse-permission");
    agent.send(&prompt_request("prompt-permission"));
    assert_eq!(agent.receive()["method"], "session/update");
    let permission = agent.receive();
    assert_eq!(permission["method"], "session/request_permission");
    assert_eq!(permission["params"]["options"][0]["kind"], "reject_once");
    agent.send(&json!({
        "jsonrpc": "2.0",
        "id": permission["id"],
        "result": {"outcome": {"outcome": "cancelled"}}
    }));
    assert_eq!(agent.receive()["result"]["stopReason"], "end_turn");
    let observation = agent.finish();
    assert!(observation.contains("\"denied\":true"));
}

#[test]
fn fake_agent_models_preflight_and_active_crashes() {
    let mut preflight = FakeAcpHarness::spawn("crash-initialize");
    preflight.send(&initialize_request("initialize-crash"));
    assert!(preflight.lines.recv_timeout(RESPONSE_DEADLINE).is_err());
    assert_eq!(preflight.wait_for_exit().code(), Some(71));

    let mut active = FakeAcpHarness::spawn("crash-prompt");
    active.send(&initialize_request("initialize-active"));
    assert_eq!(active.receive()["result"]["protocolVersion"], 1);
    active.send(&prompt_request("prompt-crash"));
    assert!(active.lines.recv_timeout(RESPONSE_DEADLINE).is_err());
    assert_eq!(active.wait_for_exit().code(), Some(72));
}

#[test]
fn fake_agent_produces_malformed_oversized_and_incompatible_fixtures() {
    let mut malformed = FakeAcpHarness::spawn("malformed");
    malformed.send(&initialize_request("initialize-malformed"));
    let raw = malformed.receive_raw();
    assert!(serde_json::from_str::<Value>(&raw).is_err());
    malformed.finish();

    let mut oversized = FakeAcpHarness::spawn("oversize");
    oversized.send(&initialize_request("initialize-oversized"));
    assert_eq!(oversized.receive_raw().len(), 8_388_609);
    oversized.finish();

    let mut incompatible = FakeAcpHarness::spawn("incompatible-version");
    incompatible.send(&initialize_request("initialize-incompatible"));
    assert_eq!(incompatible.receive()["result"]["protocolVersion"], 2);
    incompatible.finish();

    let mut missing = FakeAcpHarness::spawn("missing-capability");
    missing.send(&initialize_request("initialize-missing"));
    assert_eq!(
        missing.receive()["result"]["agentCapabilities"]["loadSession"],
        false
    );
    missing.finish();
}

#[test]
fn fake_children_are_workspace_scoped_and_independently_reaped() {
    let mut first = FakeAcpHarness::spawn("hang");
    let mut second = FakeAcpHarness::spawn("hang");
    assert_ne!(first.workspace_path(), second.workspace_path());
    assert!(first.is_running());
    assert!(second.is_running());

    let first_observation = first.finish();
    assert!(first_observation.contains("\"event\":\"started\""));
    assert!(second.is_running());

    let second_observation = second.finish();
    assert!(second_observation.contains("\"event\":\"started\""));
}

#[derive(Default)]
struct CapturingTelemetry {
    labels: Mutex<Vec<&'static str>>,
}

#[derive(Clone, Default)]
struct CapturedLogs {
    inner: Arc<Mutex<CapturedLogBuffer>>,
}

static CAPTURED_LOGS: OnceLock<CapturedLogs> = OnceLock::new();
static TRACING_INITIALIZED: Once = Once::new();

#[derive(Default)]
struct CapturedLogBuffer {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedLogs {
    fn checkpoint(&self) -> usize {
        self.inner.lock().expect("captured logs").bytes.len()
    }

    fn snapshot_since(&self, checkpoint: usize) -> String {
        let inner = self.inner.lock().expect("captured logs");
        assert!(!inner.truncated, "Workbench log capture was truncated");
        assert!(
            checkpoint <= inner.bytes.len(),
            "Workbench log checkpoint exceeds the captured buffer"
        );
        String::from_utf8(inner.bytes[checkpoint..].to_vec()).expect("Workbench logs are UTF-8")
    }
}

struct CapturedLogWriter {
    inner: Arc<Mutex<CapturedLogBuffer>>,
    event: Vec<u8>,
    truncated: bool,
}

impl Write for CapturedLogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = MAX_CAPTURED_LOG_BYTES.saturating_sub(self.event.len());
        let accepted = remaining.min(bytes.len());
        self.event.extend_from_slice(&bytes[..accepted]);
        self.truncated |= accepted < bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for CapturedLogWriter {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().expect("captured log writer");
        let remaining = MAX_CAPTURED_LOG_BYTES.saturating_sub(inner.bytes.len());
        let accepted = remaining.min(self.event.len());
        inner.bytes.extend_from_slice(&self.event[..accepted]);
        inner.truncated |= self.truncated || accepted < self.event.len();
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = CapturedLogWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        CapturedLogWriter {
            inner: Arc::clone(&self.inner),
            event: Vec::with_capacity(MAX_CAPTURED_LOG_BYTES),
            truncated: false,
        }
    }
}

fn global_workbench_logs() -> CapturedLogs {
    assert_eq!(
        std::env::var(TRACING_CHILD_ENV).as_deref(),
        Ok("1"),
        "global tracing is restricted to the dedicated Feature 004 child"
    );
    let captured = CAPTURED_LOGS.get_or_init(CapturedLogs::default).clone();
    TRACING_INITIALIZED.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(captured.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("Feature 004 global tracing subscriber");
    });
    captured
}

#[test]
fn captured_log_writer_bounds_each_event_before_publication() {
    let captured = CapturedLogs::default();
    let mut writer = captured.make_writer();
    let oversized = vec![b'x'; MAX_CAPTURED_LOG_BYTES + 1];
    writer.write_all(&oversized).expect("oversized log event");
    assert_eq!(writer.event.len(), MAX_CAPTURED_LOG_BYTES);
    assert!(writer.event.capacity() <= MAX_CAPTURED_LOG_BYTES);
    assert!(writer.truncated);
    drop(writer);

    let inner = captured.inner.lock().expect("captured oversized event");
    assert_eq!(inner.bytes.len(), MAX_CAPTURED_LOG_BYTES);
    assert!(inner.truncated);
}

impl CapturingTelemetry {
    fn snapshot(&self) -> String {
        self.labels.lock().expect("telemetry labels").join("\n")
    }
}

impl Telemetry for CapturingTelemetry {
    fn record_route(&self, selected_rule: &'static str, outcome: &'static str) {
        self.labels
            .lock()
            .expect("telemetry labels")
            .extend([selected_rule, outcome]);
    }

    fn record_attempt(&self, outcome: &'static str) {
        self.labels.lock().expect("telemetry labels").push(outcome);
    }
}

struct SutSessionOutcome {
    session: SessionResult,
    events: Vec<SessionEvent>,
    observation: String,
    workbench_boundaries: String,
    telemetry_labels: String,
    cancellation_elapsed: Option<Duration>,
}

#[allow(clippy::too_many_lines)]
async fn execute_sut_session(
    mode: &str,
    cancel: bool,
    export_for_inspection: bool,
) -> SutSessionOutcome {
    let workspace = secure_tempdir(&format!("wb-acp-sut-{mode}-"));
    set_sut_mode(workspace.path(), mode);
    let startup = acp_startup(workspace.path(), Path::new(FAKE_ACP_AGENT));
    let providers = ProviderRuntime::bootstrap(&startup, workspace.path(), workspace.path())
        .await
        .unwrap_or_else(|error| panic!("{mode} provider bootstrap failed: {error}"));
    let storage_directory = workspace.path().join("storage");
    fs::create_dir(&storage_directory).expect("private storage directory");
    fs::set_permissions(&storage_directory, fs::Permissions::from_mode(0o700))
        .expect("private storage directory permissions");
    let database = storage_directory.join("workbench.sqlite");
    let key_store = MemoryKeyStore::new();
    let storage =
        SqliteStorage::open(&database, key_store.clone()).expect("encrypted test storage");
    let telemetry = Arc::new(CapturingTelemetry::default());
    let application = Application::new_with_providers_and_telemetry(
        storage,
        startup,
        FakeBehavior::default(),
        telemetry.clone(),
        providers.registry(),
        providers.catalog(),
    );
    let daemon = LocalDaemonHarness::start(application.clone()).expect("local daemon");
    let mut controller = ProtocolTestClient::connect(daemon.endpoint(), "feature-004-sut")
        .await
        .expect("controller");
    let created = timed_call(
        &mut controller,
        client_command(
            None,
            ProtocolCommand::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
                workflow: None,
            }),
        ),
    )
    .await;
    let created: CreateSessionResult =
        serde_json::from_value(created).expect("session creation result");
    let prompted = timed_call(
        &mut controller,
        client_command(
            Some(created.session_id),
            ProtocolCommand::SessionPrompt(PromptParams {
                text: "provider boundary acceptance".to_owned(),
                explicit_target: None,
            }),
        ),
    )
    .await;
    serde_json::from_value::<PromptResult>(prompted).expect("prompt result");
    let awaiting = get_session(&mut controller, created.session_id).await;
    let approval_id = awaiting.pending_approval_id.expect("pending approval ID");
    let approved = timed_call(
        &mut controller,
        client_command(
            Some(created.session_id),
            ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                approval_id,
                decision: ApprovalDecision::Grant,
            }),
        ),
    )
    .await;
    serde_json::from_value::<ApprovalResult>(approved).expect("approval result");

    let cancel_started = if cancel {
        wait_for_observation(workspace.path(), "\"method\":\"session/prompt\"").await;
        let started = Instant::now();
        let cancelled = timed_call(
            &mut controller,
            client_command(
                Some(created.session_id),
                ProtocolCommand::SessionCancel(EmptyParams {}),
            ),
        )
        .await;
        assert!(cancelled.get("state").is_some());
        Some(started)
    } else {
        None
    };
    let session = wait_for_terminal(&mut controller, created.session_id).await;
    let cancellation_elapsed = cancel_started.map(|started| started.elapsed());
    let events = replay_all(daemon.endpoint(), created.session_id).await;

    let mut extra_boundaries =
        serde_json::to_string(&(&session, &events)).expect("protocol boundary JSON");
    if export_for_inspection {
        let identity = age::x25519::Identity::generate();
        let output_path = storage_directory.join("session.age");
        let exported = timed_call(
            &mut controller,
            client_command(
                Some(created.session_id),
                ProtocolCommand::SessionExport(ExportParams {
                    output_path: output_path.to_string_lossy().into_owned(),
                    age_recipients: vec![identity.to_public().to_string()],
                }),
            ),
        )
        .await;
        extra_boundaries
            .push_str(&serde_json::to_string(&exported).expect("export result boundary JSON"));
        let ciphertext = fs::read(&output_path).expect("encrypted export");
        let plaintext = age::decrypt(&identity, &ciphertext).expect("decrypt export");
        extra_boundaries.push_str(&String::from_utf8(plaintext).expect("export UTF-8"));
    }

    drop(controller);
    drop(daemon);
    application
        .prepare_shutdown()
        .await
        .expect("application shutdown");
    providers.shutdown().await.expect("provider child reap");
    drop(providers);
    drop(application);

    let observation = sut_observation(workspace.path());
    append_boundary_files(workspace.path(), &mut extra_boundaries);
    SutSessionOutcome {
        session,
        events,
        observation,
        workbench_boundaries: extra_boundaries,
        telemetry_labels: telemetry.snapshot(),
        cancellation_elapsed,
    }
}

async fn timed_call(client: &mut ProtocolTestClient, command: ClientCommand) -> Value {
    tokio::time::timeout(SUT_DEADLINE, client.call(command))
        .await
        .expect("protocol call deadline")
        .expect("protocol call")
}

async fn get_session(client: &mut ProtocolTestClient, session_id: Uuid) -> SessionResult {
    serde_json::from_value(
        timed_call(
            client,
            client_command(
                Some(session_id),
                ProtocolCommand::SessionGet(EmptyParams {}),
            ),
        )
        .await,
    )
    .expect("session result")
}

async fn wait_for_terminal(client: &mut ProtocolTestClient, session_id: Uuid) -> SessionResult {
    tokio::time::timeout(SUT_DEADLINE, async {
        loop {
            let session = get_session(client, session_id).await;
            if matches!(
                session.state,
                SessionState::Completed
                    | SessionState::Failed
                    | SessionState::Cancelled
                    | SessionState::OutcomeUnknown
            ) {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal session deadline")
}

async fn replay_all(endpoint: &Path, session_id: Uuid) -> Vec<SessionEvent> {
    let mut observer = ProtocolTestClient::connect(endpoint, "feature-004-audit")
        .await
        .expect("audit client");
    let attached = timed_call(
        &mut observer,
        client_command(
            Some(session_id),
            ProtocolCommand::SessionAttach(AttachSessionParams { after_sequence: 0 }),
        ),
    )
    .await;
    let attached: AttachSessionResult = serde_json::from_value(attached).expect("attach result");
    let mut events = Vec::new();
    tokio::time::timeout(SUT_DEADLINE, async {
        while events.len() < usize::try_from(attached.last_sequence).expect("bounded sequence") {
            events.push(observer.next_event().await.expect("replayed event"));
        }
    })
    .await
    .expect("event replay deadline");
    events
}

async fn wait_for_observation(workspace: &Path, needle: &str) {
    tokio::time::timeout(SUT_DEADLINE, async {
        loop {
            if sut_observation(workspace).contains(needle) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake observation deadline");
}

fn event_count(events: &[SessionEvent], kind: EventKind) -> usize {
    events.iter().filter(|event| event.kind == kind).count()
}

fn set_sut_mode(workspace: &Path, mode: &str) {
    fs::write(workspace.join(MODE_FILE), mode).expect("workspace-local fake ACP mode");
}

fn sut_observation(workspace: &Path) -> String {
    fs::read_to_string(workspace.join(OBSERVATION_FILE)).unwrap_or_default()
}

fn observation_events(workspace: &Path) -> Vec<Value> {
    sut_observation(workspace)
        .lines()
        .map(|line| serde_json::from_str(line).expect("fake observation event"))
        .collect()
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

fn request(id: &str, method: &str, params: Value) -> Value {
    let mut envelope = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": null
    });
    envelope["params"] = params;
    envelope
}

fn notification(method: &str, params: Value) -> Value {
    let mut envelope = json!({"jsonrpc": "2.0", "method": method, "params": null});
    envelope["params"] = params;
    envelope
}

fn initialize_request(id: &str) -> Value {
    request(
        id,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {
                "name": "workbench-test",
                "title": "Workbench Test",
                "version": "1"
            }
        }),
    )
}

fn prompt_request(id: &str) -> Value {
    request(
        id,
        "session/prompt",
        json!({
            "sessionId": "fake-acp-session",
            "prompt": [{"type": "text", "text": "deterministic prompt"}]
        }),
    )
}

fn acp_startup(repository_root: &Path, executable: &Path) -> StartupConfiguration {
    let inputs = BTreeMap::from([(
        "fake".to_owned(),
        AdapterInput::acp(executable, "1.0.0-test").expect("adapter input"),
    )]);
    acp_startup_with_inputs(repository_root, executable, &inputs)
}

fn acp_startup_with_inputs(
    repository_root: &Path,
    executable: &Path,
    inputs: &BTreeMap<String, AdapterInput>,
) -> StartupConfiguration {
    let executable = executable
        .canonicalize()
        .expect("canonical fake ACP executable");
    let mut configuration = WorkbenchConfiguration::safe_builtins();
    let provider = configuration
        .providers
        .get_mut("fake")
        .expect("built-in provider");
    provider.kind = ProviderType::Acp;
    provider.executable = Some(executable.to_string_lossy().into_owned());
    let configuration_directory = repository_root.join(".workbench");
    fs::create_dir_all(&configuration_directory).expect("configuration directory");
    let configuration_path = configuration_directory.join("feature-004-acceptance.yaml");
    fs::write(
        &configuration_path,
        serde_yaml_ng::to_string(&configuration).expect("configuration YAML"),
    )
    .expect("configuration file");
    fs::set_permissions(&configuration_path, fs::Permissions::from_mode(0o600))
        .expect("configuration permissions");
    let inspected = StartupConfiguration::inspect_with_adapter_inputs(
        repository_root,
        Some(&configuration_path),
        inputs,
    )
    .expect("inspect ACP configuration");
    inspected
        .write_base_lock(repository_root)
        .expect("write repository lock");
    StartupConfiguration::load_with_configuration(repository_root, Some(&configuration_path))
        .expect("load verified repository lock")
}

fn client_command(session_id: Option<Uuid>, command: ProtocolCommand) -> ClientCommand {
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
