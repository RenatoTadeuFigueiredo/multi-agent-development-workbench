use std::{
    env,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use workbench_claude::{ClaudeLaunchProfile, ClaudeProviderAdapter};
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{
        CancellationStatus, ProviderAdapter, ProviderCapability, ProviderOutput, ProviderPrompt,
    },
    value::{NonEmptyText, ProviderId},
};

const FAKE_VERSION: &str = "2.1.218";

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|argument| argument == "auth") {
        println!(
            "{}",
            json!({
                "loggedIn": true,
                "authMethod": "claude.ai",
                "apiProvider": "firstParty",
                "sensitiveAccountMarker": "discard-me"
            })
        );
        return;
    }
    if args.first().is_some_and(|argument| argument == "--version") {
        println!("{FAKE_VERSION} (Claude Code)");
        return;
    }
    if args.iter().any(|argument| argument == "--print") {
        fake_stream(&args);
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(run());
}

async fn run() {
    incompatible_version_is_reaped().await;
    normalized_read_only_stream_completes_once().await;
    active_transport_loss_is_uncertain().await;
    disallowed_native_tool_fails_closed().await;
    cancellation_requires_both_receipts().await;
    cancellation_before_process_setup_prevents_dispatch().await;
    blocked_prompt_input_times_out_and_is_reaped().await;
    concurrent_shutdown_reaps_all_children_with_one_deadline().await;
}

async fn incompatible_version_is_reaped() {
    let workspace = TempDir::new().expect("version workspace");
    let error = ClaudeProviderAdapter::connect(
        provider_id(),
        "2.1.217".to_owned(),
        profile(&workspace),
        Duration::from_millis(200),
    )
    .await
    .err()
    .expect("version mismatch");
    assert_eq!(error.category(), FailureCategory::CapabilityUnavailable);
    assert!(observation(&workspace).contains("event=eof"));
}

async fn normalized_read_only_stream_completes_once() {
    let workspace = TempDir::new().expect("contract workspace");
    let adapter = connect(&workspace, Duration::from_millis(200)).await;
    let capabilities = adapter.capabilities().await.expect("capabilities");
    assert_eq!(capabilities.adapter_version, FAKE_VERSION);
    assert_eq!(capabilities.protocol, "claude-code-stream-json/1");
    assert_eq!(
        capabilities.capabilities,
        vec![
            ProviderCapability::Streaming,
            ProviderCapability::Cancellation
        ]
    );
    assert!(
        !capabilities
            .capabilities
            .contains(&ProviderCapability::ToolCalling)
    );

    let handle = adapter.start_session().await.expect("session");
    assert!(
        adapter
            .resume_session(handle.expose_to_adapter())
            .await
            .is_err()
    );
    let mut stream = adapter
        .prompt_stream(&handle, prompt("stream"))
        .await
        .expect("prompt stream");
    let mut outputs = Vec::new();
    while let Some(output) = stream.next().await {
        outputs.push(output.expect("normalized output"));
    }
    assert_eq!(
        outputs
            .iter()
            .filter(|output| matches!(output, ProviderOutput::Acknowledged { .. }))
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|output| matches!(output, ProviderOutput::Content { .. }))
            .count(),
        1,
        "the final assistant copy must not duplicate the partial text"
    );
    assert!(outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::Tool { content, .. } if content.as_str() == "Read"
    )));
    assert_eq!(
        outputs
            .iter()
            .filter(|output| matches!(output, ProviderOutput::Completed { .. }))
            .count(),
        1
    );

    let observed = observation(&workspace);
    for required in [
        "--output-format=stream-json",
        "--input-format=stream-json",
        "--permission-mode=dontAsk",
        "--tools=Read,Glob,Grep",
        "--allowedTools=Read,Glob,Grep",
        "--safe-mode",
        "--disable-slash-commands",
        "--no-chrome",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--mcp-config={\"mcpServers\":{}}",
        "--setting-sources=",
        "autoupdater=1",
    ] {
        assert!(
            observed.contains(required),
            "missing observation: {required}"
        );
    }
    assert!(adapter.shutdown().await.reaped);
}

async fn active_transport_loss_is_uncertain() {
    let workspace = TempDir::new().expect("crash workspace");
    let adapter = connect(&workspace, Duration::from_millis(200)).await;
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("crash"))
        .await
        .expect("active prompt");
    assert!(matches!(
        stream.next().await,
        Some(Ok(ProviderOutput::Acknowledged { .. }))
    ));
    let failure = stream
        .next()
        .await
        .expect("terminal item")
        .expect_err("crash must be uncertain");
    assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
    assert!(!failure.definite);
    assert!(adapter.shutdown().await.reaped);
}

async fn disallowed_native_tool_fails_closed() {
    let workspace = TempDir::new().expect("tool workspace");
    let adapter = connect(&workspace, Duration::from_millis(200)).await;
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("denied-tool"))
        .await
        .expect("active prompt");
    assert!(stream.next().await.is_some());
    let failure = stream
        .next()
        .await
        .expect("terminal item")
        .expect_err("disallowed tool");
    assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
    assert!(!failure.definite);
    assert!(adapter.shutdown().await.reaped);
}

async fn cancellation_requires_both_receipts() {
    for (text, expected) in [
        ("wait-for-cancel", CancellationStatus::Confirmed),
        ("interrupt-ack-only", CancellationStatus::Unconfirmed),
        (
            "aborted-before-interrupt-ack",
            CancellationStatus::Unconfirmed,
        ),
    ] {
        let workspace = TempDir::new().expect("cancellation workspace");
        let adapter = connect(&workspace, Duration::from_millis(100)).await;
        let handle = adapter.start_session().await.expect("session");
        let prompt = prompt(text);
        let attempt_id = prompt.attempt_id;
        let mut stream = adapter
            .prompt_stream(&handle, prompt)
            .await
            .expect("active prompt");
        assert!(matches!(
            stream.next().await,
            Some(Ok(ProviderOutput::Acknowledged { .. }))
        ));
        assert_eq!(
            adapter
                .cancel(&handle, attempt_id)
                .await
                .expect("cancellation"),
            expected
        );
        assert!(stream.next().await.is_none());
        assert!(adapter.shutdown().await.reaped);
    }
}

async fn blocked_prompt_input_times_out_and_is_reaped() {
    let workspace = TempDir::new().expect("blocked input workspace");
    let adapter = connect(&workspace, Duration::from_millis(100)).await;
    let handle = adapter.start_session().await.expect("session");
    let failure = adapter
        .prompt_stream(
            &handle,
            ProviderPrompt {
                session_id: SessionId::new(),
                attempt_id: AttemptId::new(),
                runtime_model: "block-stdin".to_owned(),
                content: NonEmptyText::parse("x".repeat(1024 * 1024)).expect("large prompt"),
            },
        )
        .await
        .err()
        .expect("blocked write must fail");
    assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
    assert!(!failure.definite);
    assert!(adapter.shutdown().await.reaped);
}

async fn cancellation_before_process_setup_prevents_dispatch() {
    let workspace = TempDir::new().expect("pre-dispatch cancellation workspace");
    let adapter = connect(&workspace, Duration::from_millis(100)).await;
    let handle = adapter.start_session().await.expect("session");
    let prompt = prompt("stream");
    assert_eq!(
        adapter
            .cancel(&handle, prompt.attempt_id)
            .await
            .expect("pending cancellation"),
        CancellationStatus::Unconfirmed
    );
    let failure = adapter
        .prompt_stream(&handle, prompt)
        .await
        .err()
        .expect("cancelled setup");
    assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
    assert!(!observation(&workspace).contains("event=user"));
    assert!(adapter.shutdown().await.reaped);
}

async fn concurrent_shutdown_reaps_all_children_with_one_deadline() {
    let workspace = TempDir::new().expect("parallel shutdown workspace");
    let adapter = Arc::new(connect(&workspace, Duration::from_millis(450)).await);
    for text in ["first-wait", "second-wait"] {
        let handle = adapter.start_session().await.expect("session");
        let mut stream = adapter
            .prompt_stream(&handle, prompt(text))
            .await
            .expect("waiting prompt");
        assert!(stream.next().await.is_some());
    }
    let started = tokio::time::Instant::now();
    let report = adapter.shutdown().await;
    assert!(report.reaped);
    assert!(started.elapsed() < Duration::from_millis(450));
}

async fn connect(workspace: &TempDir, cancellation_deadline: Duration) -> ClaudeProviderAdapter {
    ClaudeProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(workspace),
        cancellation_deadline,
    )
    .await
    .expect("Claude adapter")
}

fn provider_id() -> ProviderId {
    ProviderId::parse("claude").expect("provider id")
}

fn profile(workspace: &TempDir) -> ClaudeLaunchProfile {
    ClaudeLaunchProfile::new(
        env::current_exe().expect("test executable"),
        workspace.path(),
    )
    .initialization_timeout(Duration::from_millis(500))
    .shutdown_grace(Duration::from_millis(100))
}

fn prompt(text: &str) -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: "fable".to_owned(),
        content: NonEmptyText::parse(text).expect("prompt text"),
    }
}

#[allow(clippy::too_many_lines)]
fn fake_stream(args: &[String]) {
    record_launch(args);
    let block_stdin = args
        .windows(2)
        .any(|pair| pair == ["--model", "block-stdin"]);
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    while let Some(Ok(line)) = lines.next() {
        let request: Value = serde_json::from_str(&line).expect("request JSON");
        match request.get("type").and_then(Value::as_str) {
            Some("control_request")
                if request["request"]["subtype"].as_str() == Some("initialize") =>
            {
                let request_id = request["request_id"].clone();
                emit(
                    &mut stdout,
                    &json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": request_id,
                            "response": {"commands": []}
                        }
                    }),
                );
                emit(
                    &mut stdout,
                    &json!({
                        "type": "system",
                        "subtype": "init",
                        "session_id": "provider-session-secret",
                        "claude_code_version": FAKE_VERSION,
                        "tools": ["Read", "Glob", "Grep"]
                    }),
                );
                if block_stdin {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
            Some("user") => match request["message"]["content"].as_str().unwrap_or_default() {
                "stream" => {
                    record_event("event=user");
                    emit_happy_stream(&mut stdout);
                }
                "crash" => return,
                "denied-tool" => emit(
                    &mut stdout,
                    &json!({
                        "type": "assistant",
                        "session_id": "provider-session-secret",
                        "message": {
                            "role": "assistant",
                            "content": [{
                                "type": "tool_use",
                                "name": "Bash",
                                "input": {"secretToolInput": "discard-me"}
                            }]
                        }
                    }),
                ),
                "wait-for-cancel"
                | "interrupt-ack-only"
                | "aborted-before-interrupt-ack"
                | "first-wait"
                | "second-wait" => {
                    record_event("event=prompt_waiting");
                }
                _ => unreachable!("unexpected fake prompt"),
            },
            Some("control_request")
                if request["request"]["subtype"].as_str() == Some("interrupt") =>
            {
                let request_id = request["request_id"].clone();
                let aborted_before_ack =
                    observation_text().contains("mode=aborted-before-interrupt-ack");
                if aborted_before_ack {
                    emit(
                        &mut stdout,
                        &json!({
                            "type": "result",
                            "subtype": "success",
                            "is_error": false,
                            "session_id": "provider-session-secret",
                            "terminal_reason": "aborted_streaming"
                        }),
                    );
                }
                emit(
                    &mut stdout,
                    &json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": request_id,
                            "response": {}
                        }
                    }),
                );
                if !aborted_before_ack
                    && observation_text().contains("event=prompt_waiting")
                    && !observation_text().contains("mode=interrupt-ack-only")
                {
                    emit(
                        &mut stdout,
                        &json!({
                            "type": "result",
                            "subtype": "success",
                            "is_error": false,
                            "session_id": "provider-session-secret",
                            "terminal_reason": "aborted_streaming"
                        }),
                    );
                }
            }
            _ => unreachable!("unexpected fake request"),
        }
        if request["message"]["content"].as_str() == Some("interrupt-ack-only") {
            record_event("mode=interrupt-ack-only");
        }
        if request["message"]["content"].as_str() == Some("aborted-before-interrupt-ack") {
            record_event("mode=aborted-before-interrupt-ack");
        }
    }
    record_event("event=eof");
}

fn emit_happy_stream(stdout: &mut impl Write) {
    emit(
        stdout,
        &json!({
            "type": "stream_event",
            "uuid": "event-text",
            "session_id": "provider-session-secret",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "hello"}
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "stream_event",
            "uuid": "event-thinking",
            "session_id": "provider-session-secret",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "thinking_delta", "thinking": "secretThinking"}
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "assistant",
            "session_id": "provider-session-secret",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "hello"},
                    {
                        "type": "tool_use",
                        "name": "Read",
                        "input": {"file_path": "secret-path"}
                    }
                ]
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "session_id": "provider-session-secret",
            "terminal_reason": "completed",
            "usage": {"secretUsage": true}
        }),
    );
}

fn emit(stdout: &mut impl Write, value: &Value) {
    writeln!(stdout, "{value}").expect("fake output");
    stdout.flush().expect("fake flush");
}

fn record_launch(args: &[String]) {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if matches!(
            argument.as_str(),
            "--output-format"
                | "--input-format"
                | "--permission-mode"
                | "--tools"
                | "--allowedTools"
                | "--mcp-config"
                | "--model"
        ) {
            values.push(format!(
                "{argument}={}",
                args.get(index + 1).map_or("", String::as_str)
            ));
            index += 2;
        } else {
            values.push(argument.clone());
            index += 1;
        }
    }
    values.push(format!(
        "autoupdater={}",
        env::var("DISABLE_AUTOUPDATER").unwrap_or_default()
    ));
    record_event(&values.join("\n"));
}

fn record_event(value: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("fake-claude-observation")
        .expect("observation");
    writeln!(file, "{value}").expect("observation write");
}

fn observation(workspace: &TempDir) -> String {
    std::fs::read_to_string(workspace.path().join("fake-claude-observation")).unwrap_or_default()
}

fn observation_text() -> String {
    std::fs::read_to_string(Path::new("fake-claude-observation")).unwrap_or_default()
}
