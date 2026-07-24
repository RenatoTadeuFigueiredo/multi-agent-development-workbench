use std::{env, path::PathBuf, time::Duration};

use workbench_claude::{ClaudeLaunchProfile, ClaudeProviderAdapter};
use workbench_core::{
    ports::{AuthenticationStatus, ProviderAdapter},
    value::ProviderId,
};

#[tokio::test]
#[ignore = "requires an explicit official Claude Code executable and subscription authentication"]
async fn exact_profile_initializes_without_sending_a_user_message() {
    let executable = PathBuf::from(
        env::var_os("WORKBENCH_CLAUDE_EXECUTABLE")
            .expect("WORKBENCH_CLAUDE_EXECUTABLE must select the exact executable"),
    );
    let version = env::var("WORKBENCH_CLAUDE_VERSION")
        .expect("WORKBENCH_CLAUDE_VERSION must match the selected executable");
    let workspace = env::current_dir().expect("current workspace");
    let adapter = ClaudeProviderAdapter::connect(
        ProviderId::parse("claude-live").expect("provider id"),
        version,
        ClaudeLaunchProfile::new(executable, workspace)
            .initialization_timeout(Duration::from_secs(10))
            .shutdown_grace(Duration::from_secs(2)),
        Duration::from_millis(4_500),
    )
    .await
    .expect("prompt-free Claude initialization");

    assert_eq!(
        adapter
            .authentication_status()
            .await
            .expect("authentication"),
        AuthenticationStatus::Available
    );
    assert!(adapter.shutdown().await.reaped);
}
