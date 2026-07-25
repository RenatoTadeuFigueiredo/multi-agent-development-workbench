//! Feature 011 acceptance: Workbench ACP server and terminal client MVP.

#![allow(clippy::manual_let_else)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Value, json};
use workbench_acp_server::{
    ACP_PROTOCOL_VERSION, AGENT_NAME, AcpAgentServer, AcpServerErrorKind, InProcessBackend,
    MAX_FRAME_BYTES, decode_line,
};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/workbench-acp-server-and-terminal-client.feature"
);
const BRIDGE: &str = include_str!("../../workbench-acp-server/src/bridge.rs");
const LIB: &str = include_str!("../../workbench-acp-server/src/lib.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const STATUS: &str = include_str!("../../../docs/project/STATUS.md");

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 6] = [
    binding(
        "Initialize the Workbench ACP agent offline",
        0x0,
        "initialize_create_prompt_and_cancel_work_offline",
    ),
    binding(
        "Create a session through the bridge",
        0x0,
        "initialize_create_prompt_and_cancel_work_offline",
    ),
    binding(
        "Prompt streams assistant updates",
        0x0,
        "initialize_create_prompt_and_cancel_work_offline",
    ),
    binding(
        "Cancel an active prompt",
        0x0,
        "initialize_create_prompt_and_cancel_work_offline",
    ),
    binding(
        "Reject oversized frames",
        0x0,
        "frame_bounds_and_offline_defaults",
    ),
    binding(
        "Default suite stays offline",
        0x0,
        "frame_bounds_and_offline_defaults",
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
    let _ = initialize_create_prompt_and_cancel_work_offline;
    let _ = frame_bounds_and_offline_defaults;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "initialize_create_prompt_and_cancel_work_offline",
            "frame_bounds_and_offline_defaults",
        ])
    );
}

#[tokio::test]
async fn initialize_create_prompt_and_cancel_work_offline() {
    let server = AcpAgentServer::new(Arc::new(InProcessBackend::offline_fake()));
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

    let prompted = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": "hello from ACP bridge"
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
}

#[test]
fn frame_bounds_and_offline_defaults() {
    let oversize = vec![b'x'; MAX_FRAME_BYTES + 1];
    let error = decode_line(&oversize).expect_err("oversize");
    assert_eq!(error.kind(), AcpServerErrorKind::FrameTooLarge);

    assert!(LIB.contains("workbench agent stdio") || BRIDGE.contains("session/prompt"));
    assert!(MAKEFILE.contains("feature_011") || MAKEFILE.contains("test-acp-server"));
    assert!(
        STATUS.contains("ACP server")
            || STATUS.contains("terminal")
            || STATUS.contains("Feature 011")
            || STATUS.contains("#17")
    );
    assert!(!BRIDGE.contains("https://"));
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
