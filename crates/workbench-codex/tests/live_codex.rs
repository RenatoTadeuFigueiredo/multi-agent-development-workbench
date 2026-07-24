use std::{env, path::PathBuf, time::Duration};

use workbench_codex::{CodexLaunchProfile, CodexProviderAdapter};
use workbench_core::{
    ports::{AuthenticationStatus, ProviderAdapter},
    value::ProviderId,
};

#[tokio::test]
#[ignore = "requires an explicit official Codex executable and ChatGPT subscription authentication"]
async fn prompt_free_login_status_and_version_identity_only() {
    let executable = PathBuf::from(
        env::var_os("WORKBENCH_CODEX_EXECUTABLE")
            .expect("WORKBENCH_CODEX_EXECUTABLE must select the exact executable"),
    );
    let version = env::var("WORKBENCH_CODEX_VERSION")
        .expect("WORKBENCH_CODEX_VERSION must match the selected executable");
    let workspace = env::current_dir().expect("current workspace");
    let adapter = CodexProviderAdapter::connect(
        ProviderId::parse("codex-live").expect("provider id"),
        version,
        CodexLaunchProfile::new(executable, workspace)
            .preflight_timeout(Duration::from_secs(10))
            .shutdown_grace(Duration::from_secs(2)),
        Duration::from_millis(4_500),
    )
    .await
    .expect("prompt-free Codex preflight");

    assert_eq!(
        adapter
            .authentication_status()
            .await
            .expect("authentication"),
        AuthenticationStatus::Available
    );
    assert!(adapter.shutdown().await.reaped);
}
