use std::collections::BTreeSet;

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/create-a-thin-replaceable-vs-code-extension-bridge-to-the.feature"
);
const PROTOCOL_TESTS: &str =
    include_str!("../../../extensions/workbench-vscode/src/test/protocol.test.ts");
const RENDER_TESTS: &str =
    include_str!("../../../extensions/workbench-vscode/src/test/render.test.ts");
const PROTOCOL_SOURCE: &str = include_str!("../../../extensions/workbench-vscode/src/protocol.ts");

#[test]
fn feature_002_gherkin_scenarios_are_bound_to_the_offline_extension_suite() {
    let scenarios = FEATURE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Scenario: "))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        scenarios,
        BTreeSet::from([
            "A lost transport reconnects safely",
            "Attach replays and streams a local session",
            "Markdown preview renders Mermaid",
            "Prompt and controls use the local protocol",
        ])
    );

    for evidence in [
        "negotiates, attaches, receives events, and sends prompt",
        "reconnects from the durable cursor and deduplicates replayed events",
    ] {
        assert!(
            PROTOCOL_TESTS.contains(evidence),
            "Feature 002 is missing executable evidence: {evidence}"
        );
    }
    assert!(RENDER_TESTS.contains("preserves Markdown and Mermaid"));
}

#[test]
fn feature_002_bridge_remains_a_thin_versioned_local_protocol_client() {
    for method in [
        "session.attach",
        "session.prompt",
        "session.pause",
        "session.resume",
        "session.cancel",
        "session.redirect",
    ] {
        assert!(
            PROTOCOL_SOURCE.contains(method),
            "VS Code bridge omitted {method}"
        );
    }
    assert!(PROTOCOL_SOURCE.contains("workbench/1"));
    for forbidden in ["ProviderAdapter", "OrderedRouter", "OpenRouter", "api_key"] {
        assert!(
            !PROTOCOL_SOURCE.contains(forbidden),
            "thin bridge contains forbidden orchestration concern: {forbidden}"
        );
    }
}
