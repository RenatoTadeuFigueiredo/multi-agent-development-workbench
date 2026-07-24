use std::{
    io::Write as _,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::Path,
    process::{Command, Output, Stdio},
};

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::Framed;
use uuid::Uuid;
use workbench_protocol::{ClientCommand, Command as ProtocolCommand, NdjsonCodec, PROTOCOL_V1};

const SESSION_ID: &str = "018f47ef-9052-7b86-b31d-3f8962457776";

#[test]
fn config_validate_emits_one_versioned_json_result_with_uuid_v7() {
    let root = TempDir::new().expect("temporary repository");
    let output = run(root.path(), &["--json", "config", "validate"], None);

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON result");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], true);
    let request_id = Uuid::parse_str(value["request_id"].as_str().expect("request ID"))
        .expect("UUID request ID");
    assert_eq!(request_id.get_version_num(), 7);
    assert_eq!(value["result"]["lock_written"], false);
}

#[test]
fn config_lock_writes_a_deterministic_repository_lock() {
    let root = TempDir::new().expect("temporary repository");
    let first = run(root.path(), &["--json", "config", "lock"], None);
    let lock_path = root.path().join(".workbench/workbench.lock");
    let first_lock = std::fs::read(&lock_path).expect("first lock");
    let second = run(root.path(), &["--json", "config", "lock"], None);
    let second_lock = std::fs::read(&lock_path).expect("second lock");

    assert!(first.status.success(), "{first:?}");
    assert!(second.status.success(), "{second:?}");
    assert_eq!(first_lock, second_lock);
    assert!(serde_json::from_slice::<Value>(&first_lock).is_ok());
}

#[test]
fn invalid_delete_confirmation_returns_stable_code_and_json_error() {
    let root = TempDir::new().expect("temporary repository");
    let output = run(
        root.path(),
        &[
            "--json",
            "session",
            "delete",
            SESSION_ID,
            "--confirm",
            "018f47ef-9052-7b86-b31d-3f8962457777",
        ],
        None,
    );

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON failure");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn prompt_accepts_explicit_stdin_without_echoing_it_to_diagnostics() {
    let root = TempDir::new().expect("temporary repository");
    let secret_prompt = "private prompt body";
    let output = run(
        root.path(),
        &["--json", "prompt", SESSION_ID, "-"],
        Some(secret_prompt),
    );

    assert_eq!(output.status.code(), Some(7));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret_prompt));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret_prompt));
}

#[test]
fn explicit_configuration_is_rejected_for_live_daemon_commands() {
    let root = TempDir::new().expect("temporary repository");
    let configuration = root.path().join("explicit.yaml");
    std::fs::write(&configuration, "version: 1\n").expect("configuration fixture");
    let output = run(
        root.path(),
        &[
            "--json",
            "--configuration",
            configuration.to_str().expect("UTF-8 path"),
            "status",
        ],
        None,
    );

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON failure");
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_on_an_owned_prompt_sends_session_cancel() {
    let root = tempfile::Builder::new()
        .prefix("wb-cli-")
        .tempdir_in("/tmp")
        .expect("short temporary repository");
    let endpoint = runtime_endpoint(root.path());
    std::fs::create_dir_all(endpoint.parent().expect("endpoint parent"))
        .expect("runtime directory");
    std::fs::set_permissions(
        endpoint.parent().expect("endpoint parent"),
        std::fs::Permissions::from_mode(0o700),
    )
    .expect("private runtime directory");
    let listener = UnixListener::bind(&endpoint).expect("test daemon endpoint");
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
        .expect("private endpoint");
    let session_id = Uuid::parse_str(SESSION_ID).expect("session ID");
    let (prompt_seen_sender, prompt_seen_receiver) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (prompt_stream, _) = listener.accept().await.expect("prompt client");
        let mut prompt_transport: TestTransport =
            Framed::new(prompt_stream, NdjsonCodec::default());
        initialize(&mut prompt_transport).await;
        let prompt = prompt_transport
            .next()
            .await
            .expect("prompt frame")
            .expect("prompt command");
        assert!(matches!(prompt.command, ProtocolCommand::SessionPrompt(_)));
        prompt_seen_sender.send(()).expect("notify prompt");

        let (cancel_stream, _) = listener.accept().await.expect("cancel client");
        let mut cancel_transport: TestTransport =
            Framed::new(cancel_stream, NdjsonCodec::default());
        initialize(&mut cancel_transport).await;
        let cancel = cancel_transport
            .next()
            .await
            .expect("cancel frame")
            .expect("cancel command");
        assert_eq!(cancel.session_id, Some(session_id));
        assert!(matches!(cancel.command, ProtocolCommand::SessionCancel(_)));
        cancel_transport
            .send(serde_json::json!({
                "protocol": PROTOCOL_V1,
                "request_id": cancel.request_id,
                "ok": true,
                "result": {
                    "control_id": Uuid::now_v7(),
                    "control": "cancel",
                    "state": "cancel_requested"
                }
            }))
            .await
            .expect("cancel result");
    });

    let mut child = test_command(
        root.path(),
        &["--json", "prompt", SESSION_ID, "wait for interrupt"],
    )
    .stdin(Stdio::null())
    .spawn()
    .expect("spawn CLI");
    if tokio::time::timeout(std::time::Duration::from_secs(5), prompt_seen_receiver)
        .await
        .is_err()
    {
        child.kill().expect("stop timed-out CLI");
        let output = child.wait_with_output().expect("timed-out CLI output");
        panic!(
            "prompt deadline; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success());
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .expect("join CLI wait")
        .expect("CLI output");

    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).expect("cancel JSON");
    assert_eq!(value["result"]["control"], "cancel");
    server.await.expect("server task");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_race_returns_the_completed_prompt_when_cancellation_is_too_late() {
    let root = tempfile::Builder::new()
        .prefix("wb-cli-race-")
        .tempdir_in("/tmp")
        .expect("short temporary repository");
    let endpoint = runtime_endpoint(root.path());
    std::fs::create_dir_all(endpoint.parent().expect("endpoint parent"))
        .expect("runtime directory");
    std::fs::set_permissions(
        endpoint.parent().expect("endpoint parent"),
        std::fs::Permissions::from_mode(0o700),
    )
    .expect("private runtime directory");
    let listener = UnixListener::bind(&endpoint).expect("test daemon endpoint");
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
        .expect("private endpoint");
    let (prompt_seen_sender, prompt_seen_receiver) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (prompt_stream, _) = listener.accept().await.expect("prompt client");
        let mut prompt_transport: TestTransport =
            Framed::new(prompt_stream, NdjsonCodec::default());
        initialize(&mut prompt_transport).await;
        let prompt = prompt_transport
            .next()
            .await
            .expect("prompt frame")
            .expect("prompt command");
        prompt_seen_sender.send(()).expect("notify prompt");

        let (cancel_stream, _) = listener.accept().await.expect("cancel client");
        let mut cancel_transport: TestTransport =
            Framed::new(cancel_stream, NdjsonCodec::default());
        initialize(&mut cancel_transport).await;
        let cancel = cancel_transport
            .next()
            .await
            .expect("cancel frame")
            .expect("cancel command");
        cancel_transport
            .send(serde_json::json!({
                "protocol": PROTOCOL_V1,
                "request_id": cancel.request_id,
                "ok": false,
                "error": {
                    "code": "invalid_transition",
                    "message": "session is already terminal",
                    "retryable": false,
                    "correlation_id": Uuid::now_v7()
                }
            }))
            .await
            .expect("late cancellation result");
        prompt_transport
            .send(serde_json::json!({
                "protocol": PROTOCOL_V1,
                "request_id": prompt.request_id,
                "ok": true,
                "result": {
                    "input_id": Uuid::now_v7(),
                    "sequence": 4
                }
            }))
            .await
            .expect("completed prompt result");
    });

    let child = test_command(
        root.path(),
        &["--json", "prompt", SESSION_ID, "complete during interrupt"],
    )
    .stdin(Stdio::null())
    .spawn()
    .expect("spawn CLI");
    tokio::time::timeout(std::time::Duration::from_secs(5), prompt_seen_receiver)
        .await
        .expect("prompt deadline")
        .expect("prompt notification");
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success());
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .expect("join CLI wait")
        .expect("CLI output");

    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).expect("prompt JSON");
    assert_eq!(value["result"]["sequence"], 4);
    server.await.expect("server task");
}

fn run(repository: &Path, arguments: &[&str], stdin: Option<&str>) -> Output {
    let mut command = test_command(repository, arguments);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(input) = stdin {
        command.stdin(Stdio::piped());
        let mut child = command.spawn().expect("spawn CLI");
        child
            .stdin
            .take()
            .expect("child stdin")
            .write_all(input.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("CLI output")
    } else {
        command.stdin(Stdio::null());
        command.output().expect("CLI output")
    }
}

fn test_command(repository: &Path, arguments: &[&str]) -> Command {
    let home = repository.join("home");
    let runtime = repository.join("runtime");
    std::fs::create_dir_all(&home).expect("test home");
    std::fs::create_dir_all(&runtime).expect("test runtime");
    let home = home.canonicalize().expect("canonical test home");
    let runtime = runtime.canonicalize().expect("canonical test runtime");
    let mut command = Command::new(env!("CARGO_BIN_EXE_workbench"));
    command
        .args(arguments)
        .current_dir(repository)
        .env("HOME", &home)
        .env("TMPDIR", &runtime)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn runtime_endpoint(repository: &Path) -> std::path::PathBuf {
    let runtime = repository.join("runtime");
    #[cfg(target_os = "macos")]
    {
        let uid = std::fs::metadata(repository)
            .expect("repository metadata")
            .uid();
        runtime
            .join(format!("workbench-{uid}"))
            .join("workbench.sock")
    }
    #[cfg(target_os = "linux")]
    {
        runtime.join("workbench").join("workbench.sock")
    }
}

type TestTransport = Framed<UnixStream, NdjsonCodec<ClientCommand, Value>>;

async fn initialize(transport: &mut TestTransport) {
    let initialize = transport
        .next()
        .await
        .expect("initialize frame")
        .expect("initialize command");
    assert!(matches!(initialize.command, ProtocolCommand::Initialize(_)));
    transport
        .send(serde_json::json!({
            "protocol": PROTOCOL_V1,
            "request_id": initialize.request_id,
            "ok": true,
            "result": {
                "selected_protocol": PROTOCOL_V1,
                "max_frame_bytes": 8_388_608,
                "max_client_queue_events": 1_024,
                "max_client_queue_bytes": 8_388_608
            }
        }))
        .await
        .expect("initialize result");
}
