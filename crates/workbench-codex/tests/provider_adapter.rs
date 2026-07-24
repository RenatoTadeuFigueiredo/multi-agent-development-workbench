use std::{
    env, fs::OpenOptions, io::Write, path::Path, process::Command as StdCommand, sync::Arc,
    time::Duration,
};

use futures_util::StreamExt;
use tempfile::TempDir;
use workbench_codex::{CodexLaunchProfile, CodexProviderAdapter};
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{
        CancellationStatus, ProviderAdapter, ProviderCapability, ProviderOutput, ProviderPrompt,
    },
    value::{NonEmptyText, ProviderId},
};

const FAKE_VERSION: &str = "0.145.0";

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|argument| argument == "--version") {
        println!("codex-cli {FAKE_VERSION}");
        return;
    }
    if args.first().is_some_and(|argument| argument == "login") {
        println!("Logged in using ChatGPT");
        return;
    }
    if args.first().is_some_and(|argument| argument == "exec") {
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
    cancellation_from_abort_event_is_confirmed().await;
    unconfirmed_cancellation_reaps_child().await;
    concurrent_shutdown_reaps_all_children_with_one_deadline().await;
}

async fn incompatible_version_is_reaped() {
    let workspace = TempDir::new().expect("version workspace");
    write_mode(&workspace, "happy");
    let error = CodexProviderAdapter::connect(
        provider_id(),
        "0.144.0".to_owned(),
        profile(&workspace),
        Duration::from_millis(200),
    )
    .await
    .err()
    .expect("version mismatch");
    assert_eq!(error.category(), FailureCategory::CapabilityUnavailable);
}

async fn normalized_read_only_stream_completes_once() {
    let workspace = TempDir::new().expect("contract workspace");
    write_mode(&workspace, "happy");
    let adapter = connect(&workspace, Duration::from_millis(500)).await;
    let capabilities = adapter.capabilities().await.expect("capabilities");
    assert_eq!(capabilities.adapter_version, FAKE_VERSION);
    assert_eq!(capabilities.protocol, "codex-exec-jsonl/1");
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
        1
    );
    assert!(outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::Tool { content, .. } if content.as_str() == "command_execution"
    )));
    assert_eq!(
        outputs
            .iter()
            .filter(|output| matches!(output, ProviderOutput::Completed { .. }))
            .count(),
        1
    );

    let observed = observation(&workspace);
    for required in ["exec", "--json", "--ephemeral", "--sandbox=read-only"] {
        assert!(
            observed.contains(required),
            "missing launch observation {required}: {observed}"
        );
    }
    assert!(!observed.contains("forbidden="));
    assert!(observed.contains("OPENAI_API_KEY_present=false"));
    assert!(observed.contains("CODEX_API_KEY_present=false"));
}

async fn active_transport_loss_is_uncertain() {
    let workspace = TempDir::new().expect("crash workspace");
    write_mode(&workspace, "crash");
    let adapter = connect(&workspace, Duration::from_millis(500)).await;
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("crash"))
        .await
        .expect("prompt stream");
    let mut saw_uncertain = false;
    while let Some(output) = stream.next().await {
        if let Err(failure) = output {
            assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
            assert!(!failure.definite);
            saw_uncertain = true;
        }
    }
    assert!(saw_uncertain);
}

async fn disallowed_native_tool_fails_closed() {
    let workspace = TempDir::new().expect("tool workspace");
    write_mode(&workspace, "denied-tool");
    let adapter = connect(&workspace, Duration::from_millis(500)).await;
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("tool"))
        .await
        .expect("prompt stream");
    let mut saw_failure = false;
    while let Some(output) = stream.next().await {
        if let Err(failure) = output {
            assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
            saw_failure = true;
        }
    }
    assert!(saw_failure);
}

async fn cancellation_from_abort_event_is_confirmed() {
    let workspace = TempDir::new().expect("cancel workspace");
    write_mode(&workspace, "wait-cancel");
    let adapter = Arc::new(connect(&workspace, Duration::from_millis(1_500)).await);
    let handle = adapter.start_session().await.expect("session");
    let prompt = prompt("cancel");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("prompt stream");
    let status = adapter
        .cancel(&handle, attempt_id)
        .await
        .expect("cancel status");
    assert_eq!(status, CancellationStatus::Confirmed);
    while stream.next().await.is_some() {}
}

async fn unconfirmed_cancellation_reaps_child() {
    let workspace = TempDir::new().expect("unconfirmed workspace");
    write_mode(&workspace, "cancel-silence");
    let adapter = Arc::new(connect(&workspace, Duration::from_millis(300)).await);
    let handle = adapter.start_session().await.expect("session");
    let prompt = prompt("hang");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("prompt stream");
    let status = adapter
        .cancel(&handle, attempt_id)
        .await
        .expect("cancel status");
    assert_eq!(status, CancellationStatus::Unconfirmed);
    while stream.next().await.is_some() {}
    assert!(adapter.shutdown().await.reaped);
}

async fn concurrent_shutdown_reaps_all_children_with_one_deadline() {
    let workspace = TempDir::new().expect("shutdown workspace");
    write_mode(&workspace, "cancel-silence");
    let adapter = Arc::new(connect(&workspace, Duration::from_millis(500)).await);
    let handle = adapter.start_session().await.expect("session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("shutdown"))
        .await
        .expect("prompt stream");
    let report = adapter.shutdown().await;
    assert!(report.reaped);
    while stream.next().await.is_some() {}
}

async fn connect(workspace: &TempDir, cancellation_deadline: Duration) -> CodexProviderAdapter {
    CodexProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(workspace),
        cancellation_deadline,
    )
    .await
    .expect("Codex adapter")
}

fn profile(workspace: &TempDir) -> CodexLaunchProfile {
    CodexLaunchProfile::new(current_exe(), workspace.path())
        .preflight_timeout(Duration::from_millis(500))
        .shutdown_grace(Duration::from_millis(100))
}

fn current_exe() -> std::path::PathBuf {
    env::current_exe().expect("current test executable")
}

fn provider_id() -> ProviderId {
    ProviderId::parse("codex").expect("provider ID")
}

fn prompt(text: &str) -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: "gpt-5".to_owned(),
        content: NonEmptyText::parse(text).expect("prompt text"),
    }
}

fn write_mode(workspace: &TempDir, mode: &str) {
    std::fs::write(workspace.path().join(".workbench-fake-codex-mode"), mode).expect("mode file");
}

fn observation(workspace: &TempDir) -> String {
    std::fs::read_to_string(workspace.path().join(".workbench-fake-codex-observation"))
        .unwrap_or_default()
}

fn fake_stream(args: &[String]) {
    observe_launch(args);
    let mode = std::fs::read_to_string(".workbench-fake-codex-mode")
        .ok()
        .map_or_else(|| "happy".to_owned(), |value| value.trim().to_owned());
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    match mode.as_str() {
        "crash" => std::process::exit(75),
        "denied-tool" => {
            writeln!(
                stdout,
                r#"{{"type":"item.completed","item":{{"id":"i","type":"file_change","changes":[]}}}}"#
            )
            .expect("write");
        }
        "wait-cancel" => {
            writeln!(
                stdout,
                r#"{{"type":"turn.failed","error":{{"type":"cancelled","message":"aborted"}}}}"#
            )
            .expect("write");
        }
        "cancel-silence" => {
            writeln!(stdout, r#"{{"type":"turn.started"}}"#).expect("write");
            stdout.flush().expect("flush");
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => {
            writeln!(
                stdout,
                r#"{{"type":"thread.started","thread_id":"SESSION-MARKER-F006"}}
{{"type":"turn.started"}}
{{"type":"item.started","item":{{"id":"item_1","type":"command_execution","command":"ls","status":"in_progress"}}}}
{{"type":"item.completed","item":{{"id":"item_3","type":"agent_message","text":"deterministic Codex response"}}}}
{{"type":"turn.completed","usage":{{"input_tokens":1}}}}"#
            )
            .expect("happy");
        }
    }
}

fn observe_launch(args: &[String]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(".workbench-fake-codex-observation")
        .expect("observation");
    for window in args.windows(2) {
        if matches!(window[0].as_str(), "--sandbox" | "-C" | "-m") {
            writeln!(file, "{}={}", window[0], window[1]).expect("observe");
        }
    }
    for flag in ["exec", "--json", "--ephemeral"] {
        if args.iter().any(|argument| argument == flag) {
            writeln!(file, "{flag}").expect("observe");
        }
    }
    for name in ["OPENAI_API_KEY", "CODEX_API_KEY"] {
        writeln!(file, "{name}_present={}", env::var_os(name).is_some()).expect("observe");
    }
}

#[allow(dead_code)]
fn ensure_binary_is_executable(path: &Path) {
    let _ = StdCommand::new(path).arg("--version").status();
}
