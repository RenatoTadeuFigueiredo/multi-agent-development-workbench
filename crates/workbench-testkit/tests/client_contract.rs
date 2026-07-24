use workbench_testkit::contracts::verify_local_client_contract;

#[tokio::test]
async fn local_client_contract_covers_live_methods_and_event_replay() {
    let report = verify_local_client_contract()
        .await
        .expect("local client contract");

    assert_eq!(
        report.methods,
        [
            "initialize",
            "session.approval.resolve",
            "session.attach",
            "session.cancel",
            "session.create",
            "session.delete",
            "session.export",
            "session.get",
            "session.pause",
            "session.prompt",
            "session.reconcile",
            "session.redirect",
            "session.resume",
            "status.get",
        ]
        .into_iter()
        .collect()
    );
    assert!(report.observed_events >= 4);
}
