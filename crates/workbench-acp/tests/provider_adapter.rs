use std::{
    env,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::Path,
    time::Duration,
};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use workbench_acp::{GrokLaunchProfile, GrokProviderAdapter};
use workbench_core::{
    AttemptId, FailureCategory, SessionId,
    ports::{
        AuthenticationStatus, CancellationStatus, ProviderAdapter, ProviderCapability,
        ProviderOutput, ProviderPrompt, ProviderSessionHandle,
    },
    value::{NonEmptyText, ProviderId},
};

const FAKE_VERSION: &str = "9.9.9-test";

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
    incompatible_version_is_reaped().await;
    omitted_agent_version_uses_the_locked_executable_identity().await;
    initialize_has_a_bounded_deadline().await;
    provider_port_contract_is_normalized().await;
    cancellation_deadline_is_conservative().await;
}

async fn omitted_agent_version_uses_the_locked_executable_identity() {
    let workspace = TempDir::new().expect("omitted version workspace");
    std::fs::write(workspace.path().join("omit-agent-info"), b"1").expect("omitted version marker");
    let adapter = GrokProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(&workspace),
        Duration::from_millis(100),
    )
    .await
    .expect("locked executable identity");

    assert!(adapter.shutdown().await.reaped);
}

async fn incompatible_version_is_reaped() {
    let workspace = TempDir::new().expect("version workspace");
    let error = GrokProviderAdapter::connect(
        provider_id(),
        "different-version".to_owned(),
        profile(&workspace),
        Duration::from_millis(100),
    )
    .await
    .err()
    .expect("version mismatch");
    assert_eq!(error.category(), FailureCategory::CapabilityUnavailable);
    assert!(observation(&workspace).contains("event=eof"));
}

async fn initialize_has_a_bounded_deadline() {
    let workspace = TempDir::new().expect("timeout workspace");
    std::fs::write(workspace.path().join("hang-initialize"), b"1").expect("timeout marker");
    let error = GrokProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(&workspace).request_timeout(Duration::from_millis(25)),
        Duration::from_millis(100),
    )
    .await
    .err()
    .expect("initialize timeout");
    assert_eq!(error.category(), FailureCategory::ProviderTimeout);
    assert!(observation(&workspace).contains("event=eof"));
}

async fn provider_port_contract_is_normalized() {
    let workspace = TempDir::new().expect("contract workspace");
    let adapter = GrokProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(&workspace),
        Duration::from_millis(200),
    )
    .await
    .expect("provider adapter");
    verify_preflight(&adapter).await;

    let handle = adapter.start_session().await.expect("local handle");
    assert!(!observation(&workspace).contains("session/new"));
    let resumed = adapter
        .resume_session(handle.expose_to_adapter())
        .await
        .expect("known local handle");
    verify_happy_stream(&adapter, &resumed).await;
    assert!(observation(&workspace).contains("model=grok-contract"));

    verify_definite_setup_failure(&adapter).await;
    verify_confirmed_cancellation(&adapter).await;
    verify_active_loss_is_uncertain(&adapter).await;
    assert!(adapter.shutdown().await.reaped);
}

async fn verify_preflight(adapter: &GrokProviderAdapter) {
    let capabilities = adapter.capabilities().await.expect("capabilities");
    assert_eq!(capabilities.adapter_version, FAKE_VERSION);
    assert_eq!(capabilities.protocol, "acp/1");
    assert_eq!(capabilities.authentication, AuthenticationStatus::Available);
    assert!(capabilities.capabilities.contains(&ProviderCapability::Acp));
    assert!(
        capabilities
            .capabilities
            .contains(&ProviderCapability::Streaming)
    );
    assert_eq!(
        adapter.authentication_status().await.expect("auth"),
        AuthenticationStatus::Available
    );
}

async fn verify_happy_stream(adapter: &GrokProviderAdapter, handle: &ProviderSessionHandle) {
    let mut stream = adapter
        .prompt_stream(handle, prompt("grok-contract", "happy"))
        .await
        .expect("happy stream");
    let mut acknowledged = false;
    let mut content_types = Vec::new();
    let mut tool_types = Vec::new();
    let mut completed = 0;
    while let Some(item) = stream.next().await {
        match item.expect("normalized output") {
            ProviderOutput::Acknowledged { .. } => acknowledged = true,
            ProviderOutput::Content { event_type, .. } => content_types.push(event_type),
            ProviderOutput::Tool { event_type, .. } => tool_types.push(event_type),
            ProviderOutput::Completed { .. } => completed += 1,
        }
    }
    assert!(acknowledged);
    assert_eq!(content_types, ["agent_message_chunk", "plan"]);
    assert_eq!(tool_types, ["tool_call"]);
    assert_eq!(completed, 1);
}

async fn verify_definite_setup_failure(adapter: &GrokProviderAdapter) {
    let unknown = ProviderSessionHandle::new("not-a-local-handle").expect("opaque handle");
    let failure = adapter
        .prompt_stream(&unknown, prompt("grok-contract", "never-dispatched"))
        .await
        .err()
        .expect("definite local failure");
    assert!(failure.definite);

    let handle = adapter.start_session().await.expect("rejected session");
    let failure = adapter
        .prompt_stream(&handle, prompt("reject-model", "never-dispatched"))
        .await
        .err()
        .expect("definite new-session failure");
    assert!(failure.definite);
}

async fn verify_confirmed_cancellation(adapter: &GrokProviderAdapter) {
    let handle = adapter.start_session().await.expect("cancel session");
    let prompt = prompt("grok-contract", "wait-for-cancel");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("cancel stream");
    assert_eq!(
        adapter
            .cancel(&handle, attempt_id)
            .await
            .expect("cancel result"),
        CancellationStatus::Confirmed
    );
    let mut completed = false;
    let mut cancelled_failure = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(ProviderOutput::Completed { .. }) => completed = true,
            Err(failure) => {
                cancelled_failure = true;
                assert!(!failure.definite);
            }
            Ok(_) => {}
        }
    }
    assert!(!completed);
    assert!(cancelled_failure);
}

async fn verify_active_loss_is_uncertain(adapter: &GrokProviderAdapter) {
    let handle = adapter.start_session().await.expect("crash session");
    let mut stream = adapter
        .prompt_stream(&handle, prompt("grok-contract", "crash-after-prompt"))
        .await
        .expect("started crash stream");
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(item) = item {
            failure = Some(item);
        }
    }
    let failure = failure.expect("uncertain active failure");
    assert_eq!(failure.category, FailureCategory::OutcomeUnknown);
    assert!(!failure.definite);
}

async fn cancellation_deadline_is_conservative() {
    let workspace = TempDir::new().expect("cancel workspace");
    let adapter = GrokProviderAdapter::connect(
        provider_id(),
        FAKE_VERSION.to_owned(),
        profile(&workspace),
        Duration::from_millis(25),
    )
    .await
    .expect("provider adapter");
    let handle = adapter.start_session().await.expect("cancel session");
    let prompt = prompt("grok-contract", "hang-after-cancel");
    let attempt_id = prompt.attempt_id;
    let mut stream = adapter
        .prompt_stream(&handle, prompt)
        .await
        .expect("hanging stream");
    assert_eq!(
        adapter
            .cancel(&handle, attempt_id)
            .await
            .expect("cancel result"),
        CancellationStatus::Unconfirmed
    );
    assert!(adapter.shutdown().await.reaped);
    let mut uncertain = false;
    while let Some(item) = stream.next().await {
        if let Err(failure) = item {
            uncertain = !failure.definite;
        }
    }
    assert!(uncertain);
}

fn prompt(model: &str, content: &str) -> ProviderPrompt {
    ProviderPrompt {
        session_id: SessionId::new(),
        attempt_id: AttemptId::new(),
        runtime_model: model.to_owned(),
        content: NonEmptyText::parse(content).expect("prompt content"),
    }
}

fn profile(workspace: &TempDir) -> GrokLaunchProfile {
    GrokLaunchProfile::new(
        env::current_exe().expect("test executable"),
        workspace.path(),
    )
    .request_timeout(Duration::from_millis(500))
    .shutdown_grace(Duration::from_millis(500))
}

fn provider_id() -> ProviderId {
    ProviderId::parse("grok").expect("provider ID")
}

fn observation(workspace: &TempDir) -> String {
    std::fs::read_to_string(workspace.path().join("provider-observation")).unwrap_or_default()
}

struct FakeAgent {
    next_session: usize,
    pending: Option<(Value, bool)>,
}

impl FakeAgent {
    fn new() -> Self {
        Self {
            next_session: 0,
            pending: None,
        }
    }

    fn handle(&mut self, stdout: &mut impl Write, request: &Value) -> bool {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        record(&format!("method={method}"));
        match method {
            "initialize" => Self::initialize(stdout, request),
            "session/new" => self.new_session(stdout, request),
            "session/load" => {
                respond(stdout, request.get("id").expect("load id"), &json!({}));
                true
            }
            "session/prompt" => self.prompt(stdout, request),
            "session/cancel" => self.cancel(stdout),
            _ => true,
        }
    }

    fn initialize(stdout: &mut impl Write, request: &Value) -> bool {
        if Path::new("hang-initialize").exists() {
            return true;
        }
        if Path::new("omit-agent-info").exists() {
            respond(
                stdout,
                request.get("id").expect("initialize id"),
                &json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {"loadSession": true},
                    "authMethods": []
                }),
            );
            return true;
        }
        respond(
            stdout,
            request.get("id").expect("initialize id"),
            &json!({
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": true},
                "authMethods": [],
                "agentInfo": {
                    "name": "provider-contract-fake",
                    "version": FAKE_VERSION
                }
            }),
        );
        true
    }

    fn new_session(&mut self, stdout: &mut impl Write, request: &Value) -> bool {
        let model = request["params"]["_meta"]["modelId"]
            .as_str()
            .expect("model ID");
        record(&format!("model={model}"));
        if model == "reject-model" {
            respond_error(stdout, request.get("id").expect("new id"));
            return true;
        }
        self.next_session += 1;
        respond(
            stdout,
            request.get("id").expect("new id"),
            &json!({"sessionId": format!("fake-session-{}", self.next_session)}),
        );
        true
    }

    fn prompt(&mut self, stdout: &mut impl Write, request: &Value) -> bool {
        let text = request["params"]["prompt"][0]["text"]
            .as_str()
            .expect("prompt text");
        match text {
            "happy" => {
                happy_updates(stdout, request);
                respond(
                    stdout,
                    request.get("id").expect("prompt id"),
                    &json!({"stopReason": "end_turn"}),
                );
                true
            }
            "wait-for-cancel" => {
                self.pending = request.get("id").cloned().map(|id| (id, true));
                true
            }
            "hang-after-cancel" => {
                self.pending = request.get("id").cloned().map(|id| (id, false));
                true
            }
            "crash-after-prompt" => false,
            _ => panic!("unexpected prompt"),
        }
    }

    fn cancel(&mut self, stdout: &mut impl Write) -> bool {
        if let Some((id, confirms)) = self.pending.take()
            && confirms
        {
            respond(stdout, &id, &json!({"stopReason": "cancelled"}));
        }
        true
    }
}

fn fake_agent() {
    assert_eq!(
        env::args().skip(1).collect::<Vec<_>>(),
        ["agent", "--no-leader", "stdio"]
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut agent = FakeAgent::new();
    for line in stdin.lock().lines() {
        let request: Value =
            serde_json::from_str(&line.expect("request line")).expect("request JSON");
        if !agent.handle(&mut stdout, &request) {
            return;
        }
    }
    record("event=eof");
}

fn happy_updates(stdout: &mut impl Write, request: &Value) {
    let session_id = request["params"]["sessionId"].as_str().expect("session ID");
    for update in [
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "answer"}
        }),
        json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": "private reasoning"}
        }),
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tool-1",
            "title": "denied tool"
        }),
        json!({
            "sessionUpdate": "plan",
            "entries": [{"content": "finish", "status": "in_progress"}]
        }),
    ] {
        write_json(
            stdout,
            &json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {"sessionId": session_id, "update": update}
            }),
        );
    }
}

fn respond(stdout: &mut impl Write, id: &Value, result: &Value) {
    write_json(
        stdout,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
    );
}

fn respond_error(stdout: &mut impl Write, id: &Value) {
    write_json(
        stdout,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": "rejected"}
        }),
    );
}

fn write_json(stdout: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *stdout, value).expect("response JSON");
    stdout.write_all(b"\n").expect("response newline");
    stdout.flush().expect("response flush");
}

fn record(line: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("provider-observation")
        .expect("observation file");
    writeln!(file, "{line}").expect("observation");
    file.sync_all().expect("observation sync");
}
