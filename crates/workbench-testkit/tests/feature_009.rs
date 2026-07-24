//! Feature 009 — real-time VS Code workflow controls (offline acceptance).

use std::collections::BTreeSet;

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/add-real-time-vs-code-workflow-controls-that-show-routing.feature"
);
const PROTOCOL_TESTS: &str =
    include_str!("../../../extensions/workbench-vscode/src/test/protocol.test.ts");
const RENDER_TESTS: &str =
    include_str!("../../../extensions/workbench-vscode/src/test/render.test.ts");
const PROTOCOL_SOURCE: &str = include_str!("../../../extensions/workbench-vscode/src/protocol.ts");
const RENDER_SOURCE: &str = include_str!("../../../extensions/workbench-vscode/src/render.ts");
const EXTENSION_SOURCE: &str = include_str!("../../../extensions/workbench-vscode/src/extension.ts");
const PACKAGE_JSON: &str = include_str!("../../../extensions/workbench-vscode/package.json");

#[test]
fn feature_009_gherkin_scenarios_are_bound_to_the_offline_extension_suite() {
    let scenarios = FEATURE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Scenario: "))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        scenarios,
        BTreeSet::from([
            "Approval grant and deny use the versioned protocol",
            "Lifecycle controls remain available during a workflow",
            "Offline suite stays free of network and credentials",
            "Reattach after restart deduplicates durable events",
            "Routing plans and workflow transitions render in the session document",
        ])
    );

    for evidence in [
        "renders routing plans with destination role model and provider",
        "renders workflow transitions with step iteration and phase",
        "renders approval requests and simple diffs",
        "control summary tracks workflow and pending approval from events",
    ] {
        assert!(
            RENDER_TESTS.contains(evidence),
            "Feature 009 is missing render evidence: {evidence}"
        );
    }
    for evidence in [
        "session.approval.resolve",
        "reconnects from the durable cursor and deduplicates replayed events",
        "negotiates, attaches, receives events, and sends prompt",
    ] {
        assert!(
            PROTOCOL_TESTS.contains(evidence),
            "Feature 009 is missing protocol evidence: {evidence}"
        );
    }
}

#[test]
fn feature_009_bridge_exposes_workflow_controls_without_orchestration() {
    for method in [
        "session.attach",
        "session.prompt",
        "session.pause",
        "session.resume",
        "session.cancel",
        "session.redirect",
        "session.approval.resolve",
    ] {
        assert!(
            PROTOCOL_SOURCE.contains(method),
            "VS Code bridge omitted {method}"
        );
    }
    assert!(RENDER_SOURCE.contains("routing_planned"));
    assert!(RENDER_SOURCE.contains("workflow_transition"));
    assert!(RENDER_SOURCE.contains("approval_requested"));
    assert!(RENDER_SOURCE.contains("renderControlSummary"));
    assert!(RENDER_SOURCE.contains("renderStatusBarText"));
    assert!(EXTENSION_SOURCE.contains("workbench.resolveApproval"));
    assert!(EXTENSION_SOURCE.contains("StatusBarItem"));
    assert!(PACKAGE_JSON.contains("workbench.resolveApproval"));
    assert!(PROTOCOL_SOURCE.contains("workbench/1"));
    for forbidden in ["ProviderAdapter", "OrderedRouter", "OpenRouter", "api_key", "Keychain"] {
        assert!(
            !PROTOCOL_SOURCE.contains(forbidden),
            "thin bridge contains forbidden orchestration concern: {forbidden}"
        );
        assert!(
            !EXTENSION_SOURCE.contains(forbidden),
            "extension host contains forbidden orchestration concern: {forbidden}"
        );
    }
}

#[test]
fn feature_009_repository_owned_gherkin_has_five_fingerprinted_cases() {
    let cases: Vec<_> = FEATURE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Scenario: "))
        .collect();
    assert_eq!(cases.len(), 5);
    let fingerprint = cases.iter().fold(0u64, |acc, case| {
        acc.wrapping_mul(31).wrapping_add(case.len() as u64)
    });
    assert_eq!(fingerprint, 65_265_033);
}
