use std::{env, path::PathBuf, time::Duration};

use workbench_acp::{AdapterHealth, GrokAcpClient, GrokLaunchProfile};

#[tokio::test]
#[ignore = "requires an explicitly selected, installed, and authenticated Grok Build"]
async fn exact_profile_initializes_without_starting_a_session_or_prompt() {
    let executable = PathBuf::from(
        env::var_os("WORKBENCH_GROK_EXECUTABLE")
            .expect("WORKBENCH_GROK_EXECUTABLE must select the exact executable"),
    );
    let workspace = env::current_dir().expect("current workspace");
    let client = GrokAcpClient::connect(
        GrokLaunchProfile::new(executable, workspace)
            .request_timeout(Duration::from_secs(10))
            .shutdown_grace(Duration::from_secs(2)),
    )
    .await
    .expect("live ACP initialize");

    assert!(client.capabilities().load_session);
    assert!(matches!(
        client.health(),
        AdapterHealth::Available | AdapterHealth::AuthenticationRequired
    ));
    assert!(client.shutdown().await.reaped);
}
