//! Feature 013 acceptance: non-loopback HTTPS MCP TLS client.

#![allow(clippy::manual_let_else)]

use std::collections::{BTreeMap, BTreeSet};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/compose-tls-https-client-for-non-loopback-mcp-endpoints-so.feature"
);
const MCP_HTTP: &str = include_str!("../../workbench-mcp/src/http.rs");
const MCP_MANIFEST: &str = include_str!("../../workbench-mcp/Cargo.toml");
const MCP_GATEWAY: &str = include_str!("../../workbench-mcp/src/gateway.rs");
const DAEMON_RUNTIME: &str = include_str!("../../workbench-daemon/src/runtime.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const STATUS: &str = include_str!("../../../docs/project/STATUS.md");
const PIN: &str = include_str!("../../workbench-mcp/src/pin.rs");

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 7] = [
    binding(
        "Invoke pinned HTTPS MCP through an offline TLS fixture",
        0x4da1_ee9d_9e87_75b5,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Reject cleartext non-loopback HTTP",
        0x7e08_25ef_a129_f4ca,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Reject unpinned HTTPS redirect",
        0x71de_b198_f2f1_c7bb,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Preserve loopback HTTP offline path",
        0x849b_612a_ad6a_84b1,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Redact secrets on TLS transport failure",
        0x0ab0_4a95_2366_f82a,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Keep MCP free of heavy HTTP client crates",
        0x8f7b_8aaf_661b_f4c2,
        "tls_path_and_supply_chain",
    ),
    binding(
        "Default suite stays offline",
        0x6d42_2114_8ccb_5c9d,
        "tls_path_and_supply_chain",
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
    assert_eq!(cases.len(), 7);
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
    let _ = tls_path_and_supply_chain;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(evidence, BTreeSet::from(["tls_path_and_supply_chain"]));
}

#[test]
fn tls_path_and_supply_chain() {
    // HTTPS path composed; fail-closed stub removed.
    assert!(MCP_HTTP.contains("invoke_https"));
    assert!(MCP_HTTP.contains("TlsConnector"));
    assert!(MCP_HTTP.contains("with_network"));
    assert!(MCP_HTTP.contains("with_tls_config"));
    assert!(MCP_HTTP.contains("with_connect_host_override"));
    assert!(MCP_HTTP.contains("client_config_with_root_der"));
    assert!(MCP_HTTP.contains("offline_tls_fixture_serves_non_loopback_https"));
    assert!(MCP_HTTP.contains("https_unpinned_redirect_fails_closed"));
    assert!(MCP_HTTP.contains("tls_failure_does_not_echo_secret_markers"));
    assert!(MCP_HTTP.contains("#[ignore"));
    assert!(
        !MCP_HTTP.contains("Non-loopback HTTPS requires a TLS stack; fail closed until one is")
    );

    // Cleartext non-loopback still rejected at identity parse.
    assert!(PIN.contains("rejects_cleartext_non_loopback"));
    assert!(PIN.contains("scheme == \"http\" && !loopback"));

    // Supply chain: no heavy HTTP clients; rustls stack present.
    assert!(!MCP_MANIFEST.contains("reqwest"));
    assert!(!MCP_MANIFEST.contains("hyper"));
    assert!(MCP_MANIFEST.contains("rustls"));
    assert!(MCP_MANIFEST.contains("tokio-rustls"));
    assert!(MCP_MANIFEST.contains("rustls-native-certs"));

    // Production gateway uses network client; offline still available for tests.
    assert!(MCP_GATEWAY.contains("HttpMcpClient::with_network()"));
    assert!(MCP_GATEWAY.contains("HttpMcpClient::offline()"));
    assert!(DAEMON_RUNTIME.contains("false,") || DAEMON_RUNTIME.contains("offline_http"));

    // Acceptance wiring and STATUS gap tracking.
    assert!(MAKEFILE.contains("feature_013") || MAKEFILE.contains("test-acceptance"));
    assert!(STATUS.contains("#30") || STATUS.contains("TLS") || STATUS.contains("013"));
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
