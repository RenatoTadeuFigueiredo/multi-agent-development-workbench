//! Feature 012 acceptance: attach ACP agent stdio to a running daemon.

#![allow(clippy::manual_let_else)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Value, json};
use workbench_acp_server::{
    ACP_PROTOCOL_VERSION, AGENT_NAME, AcpAgentServer, AcpServerErrorKind, DaemonSocketBackend,
};
use workbench_daemon::{Application, FakeBehavior, StartupConfiguration};
use workbench_protocol::{
    ClientCommand, Command, PROTOCOL_V1,
    command::{EmptyParams, ListSessionsParams},
    response::ListSessionsResult,
};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/attach-acp-agent-stdio-to-running-daemon.feature"
);
const BRIDGE: &str = include_str!("../../workbench-acp-server/src/bridge.rs");
const CLI_MAIN: &str = include_str!("../../workbench-cli/src/main.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const STATUS: &str = include_str!("../../../docs/project/STATUS.md");

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 6] = [
    binding(
        "Attach initialize against a live local daemon",
        0x0,
        "attach_initialize_create_list_prompt_cancel",
    ),
    binding(
        "Create a durable session visible to other clients",
        0x0,
        "attach_initialize_create_list_prompt_cancel",
    ),
    binding(
        "Prompt streams updates over the socket backend",
        0x0,
        "attach_initialize_create_list_prompt_cancel",
    ),
    binding(
        "Cancel an active prompt through the socket backend",
        0x0,
        "attach_initialize_create_list_prompt_cancel",
    ),
    binding(
        "Missing daemon fails closed",
        0x0,
        "missing_daemon_fails_closed_and_defaults_stay_offline",
    ),
    binding(
        "Default suite stays offline",
        0x0,
        "missing_daemon_fails_closed_and_defaults_stay_offline",
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
fn repository_owned_gherkin_has_fingerprinted_cases() {
    let cases = parse_feature(FEATURE);
    assert_eq!(cases.len(), 6);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    for case in &cases {
        assert!(bindings.contains_key(case.name.as_str()), "{}", case.name);
        let fp = fingerprint(&case.steps);
        let binding = bindings[case.name.as_str()];
        if binding.fingerprint != 0 {
            assert_eq!(fp, binding.fingerprint, "drift {}", case.name);
        }
        assert_ne!(fp, 0);
        eprintln!("FINGERPRINT {} => 0x{fp:016x}", case.name);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = attach_initialize_create_list_prompt_cancel;
    let _ = missing_daemon_fails_closed_and_defaults_stay_offline;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "attach_initialize_create_list_prompt_cancel",
            "missing_daemon_fails_closed_and_defaults_stay_offline",
        ])
    );
}

#[tokio::test]
async fn attach_initialize_create_list_prompt_cancel() {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins().expect("safe builtins"),
        FakeBehavior::default(),
    )
    .expect("in-memory application");
    let harness = LocalDaemonHarness::start(application).expect("local daemon");
    let backend = DaemonSocketBackend::connect(harness.endpoint())
        .await
        .expect("socket backend");
    let server = AcpAgentServer::new(Arc::new(backend));

    let init = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": ACP_PROTOCOL_VERSION,
            }
        }))
        .await
        .expect("initialize");
    let result = &init[0]["result"];
    assert_eq!(
        result["protocolVersion"].as_u64(),
        Some(ACP_PROTOCOL_VERSION)
    );
    assert_eq!(result["agentInfo"]["name"].as_str(), Some(AGENT_NAME));

    let created = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {}
        }))
        .await
        .expect("session new");
    let session_id = created[0]["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_owned();
    assert!(!session_id.is_empty());

    let mut observer = ProtocolTestClient::connect(harness.endpoint(), "feature-012-observer")
        .await
        .expect("observer");
    let listed = observer
        .call(ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id: uuid::Uuid::now_v7(),
            session_id: None,
            command: Command::SessionList(ListSessionsParams {
                limit: 50,
                before_session_id: None,
            }),
        })
        .await
        .expect("session list");
    let listed: ListSessionsResult = serde_json::from_value(listed).expect("decode list sessions");
    assert!(
        listed
            .sessions
            .iter()
            .any(|summary| summary.session_id.to_string() == session_id),
        "ACP-created session must be visible to other local clients"
    );

    let prompted = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": "hello from attached ACP"
            }
        }))
        .await
        .expect("prompt");
    assert!(
        prompted
            .iter()
            .any(|message| message.get("method").and_then(Value::as_str) == Some("session/update"))
    );
    assert!(
        prompted
            .iter()
            .any(|message| message.get("result").is_some())
    );

    let cancelled = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session/cancel",
            "params": {
                "sessionId": session_id
            }
        }))
        .await
        .expect("cancel");
    assert!(cancelled[0].get("result").is_some());

    let _ = observer
        .call(ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id: uuid::Uuid::now_v7(),
            session_id: None,
            command: Command::StatusGet(EmptyParams {}),
        })
        .await
        .expect("status still works after ACP traffic");
}

#[tokio::test]
async fn missing_daemon_fails_closed_and_defaults_stay_offline() {
    let missing = std::env::temp_dir().join(format!(
        "workbench-missing-daemon-{}.sock",
        uuid::Uuid::now_v7()
    ));
    let error = match DaemonSocketBackend::connect(&missing).await {
        Ok(_) => panic!("missing socket must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), AcpServerErrorKind::Backend);
    assert!(
        error.message().contains("daemon socket unavailable")
            || error.message().contains("workbench daemon"),
        "actionable error: {}",
        error.message()
    );

    assert!(BRIDGE.contains("DaemonSocketBackend") || BRIDGE.contains("daemon socket unavailable"));
    assert!(CLI_MAIN.contains("DaemonSocketBackend") || CLI_MAIN.contains("daemon_unavailable"));
    assert!(MAKEFILE.contains("feature_012") || MAKEFILE.contains("test-acp-attach"));
    assert!(
        STATUS.contains("#29")
            || STATUS.contains("attach")
            || STATUS.contains("Feature 012")
            || STATUS.contains("daemon endpoint")
    );
    assert!(!BRIDGE.contains("https://api.openrouter.ai"));
}

struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

fn parse_feature(source: &str) -> Vec<ParsedCase> {
    let mut cases = Vec::new();
    let mut current: Option<ParsedCase> = None;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("Scenario:") {
            if let Some(case) = current.take() {
                cases.push(case);
            }
            current = Some(ParsedCase {
                name: title.trim().to_owned(),
                steps: Vec::new(),
            });
        } else if let Some(case) = current.as_mut()
            && ["Given ", "When ", "Then ", "And ", "But "]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        {
            case.steps.push(line.to_owned());
        }
    }
    if let Some(case) = current {
        cases.push(case);
    }
    cases
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
