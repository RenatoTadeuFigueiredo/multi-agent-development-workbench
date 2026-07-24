use std::{
    env,
    io::{BufRead, Write},
    path::Path,
    time::Duration,
};

use serde_json::{Value, json};
use tempfile::TempDir;
use workbench_acp::{
    CancellationOutcome, GrokAcpClient, GrokLaunchProfile, PromptEvent, StopReason, UpdateKind,
};

fn main() {
    if env::args().nth(1).as_deref() == Some("agent") {
        fake_agent();
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(run());
}

async fn run() {
    let workspace = TempDir::new().expect("workspace");
    let executable = env::current_exe().expect("test executable");
    let client = GrokAcpClient::connect(
        GrokLaunchProfile::new(&executable, workspace.path())
            .shutdown_grace(Duration::from_secs(1)),
    )
    .await
    .expect("ACP client");

    let observed = std::fs::read_to_string(workspace.path().join("fake-observation"))
        .expect("launch observation");
    assert!(observed.contains("argv=agent,--no-leader,stdio"));
    assert!(observed.contains("autoupdater=1"));
    assert!(observed.contains(&format!("cwd={}", canonical(workspace.path()).display())));

    let session = client
        .new_session(Some("grok-test"))
        .await
        .expect("new session");
    let mut prompt = client
        .prompt(&session, "stream")
        .await
        .expect("stream prompt");
    let mut acknowledged = false;
    let mut content = false;
    let mut completed = false;
    while let Some(event) = prompt.next().await.expect("prompt event") {
        match event {
            PromptEvent::Update(update) if update.kind == UpdateKind::Acknowledged => {
                acknowledged = true;
            }
            PromptEvent::Update(update) if update.kind == UpdateKind::AgentMessage => {
                content = true;
            }
            PromptEvent::Finished(outcome) => {
                assert_eq!(outcome.stop_reason, StopReason::EndTurn);
                completed = true;
            }
            PromptEvent::Update(_) => {}
        }
    }
    assert!(acknowledged && content && completed);

    let mut permission_prompt = client
        .prompt(&session, "permission")
        .await
        .expect("permission prompt");
    while let Some(event) = permission_prompt.next().await.expect("permission event") {
        if matches!(event, PromptEvent::Finished(_)) {
            break;
        }
    }

    let mut cancelled_prompt = client
        .prompt(&session, "wait-for-cancel")
        .await
        .expect("cancellable prompt");
    let control = cancelled_prompt.control();
    assert_eq!(
        control.cancel(Duration::from_secs(1)).await,
        CancellationOutcome::Confirmed
    );
    while let Some(event) = cancelled_prompt.next().await.expect("cancel event") {
        if let PromptEvent::Finished(outcome) = event {
            assert_eq!(outcome.stop_reason, StopReason::Cancelled);
            break;
        }
    }

    let report = client.shutdown().await;
    assert!(report.reaped);
}

fn fake_agent() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(args, ["agent", "--no-leader", "stdio"]);
    record_launch_observation(&args);

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut pending_prompt: Option<Value> = None;

    while let Some(Ok(line)) = lines.next() {
        let request: Value = serde_json::from_str(&line).expect("request JSON");
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "initialize" => respond(
                &mut stdout,
                request.get("id").expect("initialize id"),
                &json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {"loadSession": true},
                    "authMethods": [],
                    "agentInfo": {
                        "name": "offline-fake",
                        "title": "Offline Fake",
                        "version": "1.0.0"
                    }
                }),
            ),
            "session/new" => respond(
                &mut stdout,
                request.get("id").expect("new id"),
                &json!({"sessionId": "fake-session"}),
            ),
            "session/load" => respond(&mut stdout, request.get("id").expect("load id"), &json!({})),
            "session/prompt" => {
                let text = request["params"]["prompt"][0]["text"]
                    .as_str()
                    .expect("prompt text");
                if text == "wait-for-cancel" {
                    pending_prompt = request.get("id").cloned();
                } else if text == "permission" {
                    write_json(
                        &mut stdout,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": "permission-request",
                            "method": "session/request_permission",
                            "params": {
                                "sessionId": "fake-session",
                                "toolCall": {"toolCallId": "tool-1"},
                                "options": []
                            }
                        }),
                    );
                    let response: Value = serde_json::from_str(
                        &lines
                            .next()
                            .expect("permission response")
                            .expect("response line"),
                    )
                    .expect("response JSON");
                    assert_eq!(response["result"]["outcome"]["outcome"], "cancelled");
                    respond(
                        &mut stdout,
                        request.get("id").expect("prompt id"),
                        &json!({"stopReason": "end_turn"}),
                    );
                } else {
                    write_json(
                        &mut stdout,
                        &json!({
                            "jsonrpc": "2.0",
                            "method": "session/update",
                            "params": {
                                "sessionId": "fake-session",
                                "update": {
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": {"type": "text", "text": "offline"}
                                }
                            }
                        }),
                    );
                    respond(
                        &mut stdout,
                        request.get("id").expect("prompt id"),
                        &json!({"stopReason": "end_turn"}),
                    );
                }
            }
            "session/cancel" => {
                if let Some(id) = pending_prompt.take() {
                    respond(&mut stdout, &id, &json!({"stopReason": "cancelled"}));
                }
            }
            _ => {}
        }
    }
}

fn record_launch_observation(args: &[String]) {
    let cwd = env::current_dir().expect("fake cwd");
    let observation = format!(
        "argv={}\nautoupdater={}\ncwd={}\n",
        args.join(","),
        env::var("GROK_DISABLE_AUTOUPDATER").unwrap_or_default(),
        cwd.display()
    );
    std::fs::write(cwd.join("fake-observation"), observation).expect("observation");
}

fn respond(stdout: &mut impl Write, id: &Value, result: &Value) {
    write_json(
        stdout,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
    );
}

fn write_json(stdout: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *stdout, value).expect("response JSON");
    stdout.write_all(b"\n").expect("response newline");
    stdout.flush().expect("response flush");
}

fn canonical(path: &Path) -> std::path::PathBuf {
    path.canonicalize().expect("canonical path")
}
