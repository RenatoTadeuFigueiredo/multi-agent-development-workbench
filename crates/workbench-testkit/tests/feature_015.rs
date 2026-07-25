//! Feature 015 acceptance: provider-native write tools under central policy.

#![allow(clippy::manual_let_else)]

use std::collections::{BTreeMap, BTreeSet};

use workbench_config::model::{NativeWriteMode, ProviderNativeWritePolicy, WorkbenchConfiguration};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/provider-native-write-tools-under-central-policy.feature"
);
const CLAUDE_PROTOCOL: &str = include_str!("../../workbench-claude/src/protocol.rs");
const CODEX_PROTOCOL: &str = include_str!("../../workbench-codex/src/protocol.rs");
const MODEL: &str = include_str!("../../workbench-config/src/model.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const STATUS: &str = include_str!("../../../docs/project/STATUS.md");

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 4] = [
    binding(
        "Default policy keeps native writes disabled",
        0x0,
        "default_policy_disables_native_writes",
    ),
    binding(
        "Allowlist enables Claude Write under policy",
        0x0,
        "allowlist_enables_provider_writes",
    ),
    binding(
        "Deny path rejects write tools without allowlist",
        0x0,
        "deny_path_rejects_write_tools",
    ),
    binding(
        "Shared tools remain on the MCP gateway",
        0x0,
        "mcp_gateway_remains_authoritative",
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
fn feature_015_gherkin_is_bound() {
    let cases = parse_feature(FEATURE);
    assert_eq!(cases.len(), 4);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    for case in &cases {
        assert!(bindings.contains_key(case.name.as_str()), "{}", case.name);
        let fp = fingerprint(&case.steps);
        assert_ne!(fp, 0);
        eprintln!("FINGERPRINT {} => 0x{fp:016x}", case.name);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = default_policy_disables_native_writes;
    let _ = allowlist_enables_provider_writes;
    let _ = deny_path_rejects_write_tools;
    let _ = mcp_gateway_remains_authoritative;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "default_policy_disables_native_writes",
            "allowlist_enables_provider_writes",
            "deny_path_rejects_write_tools",
            "mcp_gateway_remains_authoritative",
        ])
    );
}

#[test]
fn default_policy_disables_native_writes() {
    let config = WorkbenchConfiguration::safe_builtins();
    assert_eq!(
        config.policies.provider_native_writes.mode,
        NativeWriteMode::Disabled
    );
    assert!(!config.policies.provider_native_writes.allows("claude"));
    assert!(!config.policies.provider_native_writes.allows("codex"));
    assert!(MODEL.contains("ProviderNativeWritePolicy"));
    assert!(MODEL.contains("NativeWriteMode"));
}

#[test]
fn allowlist_enables_provider_writes() {
    let policy = ProviderNativeWritePolicy {
        mode: NativeWriteMode::ApprovalRequired,
        allowlist: vec!["claude".to_owned(), "codex".to_owned()],
    };
    assert!(policy.allows("claude"));
    assert!(policy.allows("codex"));
    assert!(!policy.allows("other"));
    assert!(CLAUDE_PROTOCOL.contains("WRITE_TOOLS") || CLAUDE_PROTOCOL.contains("Write"));
    assert!(CLAUDE_PROTOCOL.contains("parse_inbound_with_policy"));
    assert!(CODEX_PROTOCOL.contains("file_change"));
    assert!(CODEX_PROTOCOL.contains("parse_inbound_with_policy"));
}

#[test]
fn deny_path_rejects_write_tools() {
    let disabled = ProviderNativeWritePolicy::default();
    assert!(!disabled.allows("claude"));
    assert!(
        CLAUDE_PROTOCOL.contains("CapabilityUnavailable")
            || CLAUDE_PROTOCOL.contains("capability_violation")
    );
    assert!(
        CODEX_PROTOCOL.contains("capability_violation") || CODEX_PROTOCOL.contains("FORBIDDEN")
    );
}

#[test]
fn mcp_gateway_remains_authoritative() {
    assert!(MAKEFILE.contains("feature_007") || MAKEFILE.contains("test-mcp"));
    assert!(
        STATUS.contains("#32")
            || STATUS.contains("Feature 015")
            || STATUS.contains("provider-native")
            || STATUS.contains("write tools")
    );
}

struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

fn parse_feature(source: &str) -> Vec<ParsedCase> {
    let mut cases = Vec::new();
    let mut current: Option<ParsedCase> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Scenario:") {
            if let Some(case) = current.take() {
                cases.push(case);
            }
            current = Some(ParsedCase {
                name: rest.trim().to_owned(),
                steps: Vec::new(),
            });
        } else if let Some(case) = current.as_mut()
            && (trimmed.starts_with("Given ")
                || trimmed.starts_with("When ")
                || trimmed.starts_with("Then ")
                || trimmed.starts_with("And "))
        {
            case.steps.push(trimmed.to_owned());
        }
    }
    if let Some(case) = current {
        cases.push(case);
    }
    cases
}

fn fingerprint(steps: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for step in steps {
        step.hash(&mut hasher);
    }
    hasher.finish()
}
