use std::collections::BTreeSet;

use workbench_testkit::contracts::verify_local_client_contract;

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/add-a-versioned-session-list-command-to-the-local-workbench.feature"
);

#[test]
fn feature_003_gherkin_has_the_committed_discovery_scenarios() {
    let actual = FEATURE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Scenario: "))
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "Continue after an exclusive cursor",
        "Create a session in VS Code",
        "List a bounded metadata-only page",
        "Select a session in VS Code",
    ]);

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn feature_003_exercises_create_list_and_attach_over_offline_ipc() {
    let report = verify_local_client_contract()
        .await
        .expect("feature 003 local client contract");

    for method in ["session.create", "session.list", "session.attach"] {
        assert!(
            report.methods.contains(method),
            "feature 003 contract did not exercise {method}"
        );
    }
}
