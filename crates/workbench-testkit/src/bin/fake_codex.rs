use std::{env, fs::OpenOptions, io::Write, path::PathBuf, thread, time::Duration};

use serde_json::{Value, json};

const VERSION: &str = "0.145.0";

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mode = std::fs::read_to_string(".workbench-fake-codex-mode")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| env::var("WORKBENCH_FAKE_CODEX_MODE").ok())
        .unwrap_or_else(|| "happy".to_owned());
    if args.first().is_some_and(|argument| argument == "--version") {
        println!("codex-cli {VERSION}");
        observe("version");
        return;
    }
    if args.first().is_some_and(|argument| argument == "login") {
        emit_auth(&mode);
        observe("login_status");
        return;
    }
    if args.first().is_some_and(|argument| argument == "exec") {
        run_exec(&args, &mode);
        return;
    }
    std::process::exit(64);
}

fn emit_auth(mode: &str) {
    let status = match mode {
        "auth-not-logged" => "Not logged in",
        "auth-api" => "Logged in using an API key",
        "auth-unknown" => "Logged in using unknown",
        _ => "Logged in using ChatGPT\nprivateAuthMarker: AUTH-MARKER-F006",
    };
    println!("{status}");
}

fn run_exec(args: &[String], mode: &str) {
    observe_launch(args);
    eprintln!("STDERR-MARKER-F006");
    observe("prompt");
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    match mode {
        "crash" => std::process::exit(75),
        "malformed-duplicate" => emit_raw(
            &mut stdout,
            r#"{"type":"turn.completed","type":"turn.completed"}"#,
        ),
        "malformed-utf8" => {
            stdout.write_all(&[0xff, b'\n']).expect("invalid UTF-8");
            stdout.flush().expect("flush invalid UTF-8");
        }
        "malformed-truncated" => {
            stdout
                .write_all(b"{\"type\":\"turn.completed\"")
                .expect("truncated frame");
            stdout.flush().expect("flush truncated frame");
        }
        "malformed-empty" => emit_raw(&mut stdout, ""),
        "malformed-envelope" => emit(
            &mut stdout,
            &json!({"type": "future_authority", "grant": true}),
        ),
        "denied-tool" => {
            emit(
                &mut stdout,
                &json!({"type": "thread.started", "thread_id": "SESSION-MARKER-F006"}),
            );
            emit(
                &mut stdout,
                &json!({
                    "type": "item.completed",
                    "item": {
                        "id": "item_write",
                        "type": "file_change",
                        "changes": [{"path": "TOOL-MARKER-F006"}]
                    }
                }),
            );
        }
        "wait-cancel" => {
            emit(
                &mut stdout,
                &json!({"type": "thread.started", "thread_id": "SESSION-MARKER-F006"}),
            );
            emit(&mut stdout, &json!({"type": "turn.started"}));
            thread::sleep(Duration::from_millis(50));
            emit(
                &mut stdout,
                &json!({
                    "type": "turn.failed",
                    "error": {
                        "type": "cancelled",
                        "message": "aborted by operator"
                    }
                }),
            );
        }
        "cancel-silence" => {
            emit(
                &mut stdout,
                &json!({"type": "thread.started", "thread_id": "SESSION-MARKER-F006"}),
            );
            emit(&mut stdout, &json!({"type": "turn.started"}));
            thread::sleep(Duration::from_secs(30));
        }
        "cancel-error" => {
            emit(
                &mut stdout,
                &json!({"type": "thread.started", "thread_id": "SESSION-MARKER-F006"}),
            );
            emit(
                &mut stdout,
                &json!({
                    "type": "turn.failed",
                    "error": {"type": "failed", "message": "provider error"}
                }),
            );
        }
        "preflight-fail" => std::process::exit(70),
        _ => emit_happy(&mut stdout),
    }
    observe("eof");
}

fn emit_happy(stdout: &mut impl Write) {
    emit(
        stdout,
        &json!({"type": "thread.started", "thread_id": "SESSION-MARKER-F006"}),
    );
    emit(stdout, &json!({"type": "turn.started"}));
    emit(
        stdout,
        &json!({
            "type": "item.started",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "TOOL-MARKER-F006",
                "status": "in_progress"
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "TOOL-MARKER-F006",
                "status": "completed"
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item_2",
                "type": "reasoning",
                "text": "THINKING-MARKER-F006"
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "item.completed",
            "item": {
                "id": "item_3",
                "type": "agent_message",
                "text": "deterministic Codex response"
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "turn.completed",
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "privateUsageMarker": "USAGE-MARKER-F006"
            }
        }),
    );
}

fn emit(stdout: &mut impl Write, value: &Value) {
    writeln!(stdout, "{value}").expect("fake response");
    stdout.flush().expect("fake response flush");
}

fn emit_raw(stdout: &mut impl Write, value: &str) {
    writeln!(stdout, "{value}").expect("fake raw response");
    stdout.flush().expect("fake raw response flush");
}

fn observe_launch(args: &[String]) {
    for window in args.windows(2) {
        if matches!(
            window[0].as_str(),
            "--sandbox" | "-C" | "--cd" | "-m" | "--model"
        ) {
            observe(&format!("{}={}", window[0], window[1]));
        }
    }
    for flag in ["exec", "--json", "--ephemeral"] {
        if args.iter().any(|argument| argument == flag) {
            observe(flag);
        }
    }
    for forbidden in [
        "--dangerously-bypass-approvals-and-sandbox",
        "--full-auto",
        "--oss",
        "workspace-write",
        "danger-full-access",
    ] {
        if args.iter().any(|argument| argument == forbidden) {
            observe(&format!("forbidden={forbidden}"));
        }
    }
    for name in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "OPENAI_BASE_URL",
        "OLLAMA_BASE_URL",
    ] {
        observe(&format!("{name}_present={}", env::var_os(name).is_some()));
    }
    observe(&format!(
        "env_marker_present={}",
        env::var_os("ENV_MARKER_F006").is_some()
    ));
}

fn observe(event: &str) {
    let path = env::var_os("WORKBENCH_FAKE_CODEX_OBSERVATION").map_or_else(
        || PathBuf::from(".workbench-fake-codex-observation"),
        PathBuf::from,
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("fake observation");
    writeln!(file, "{event}").expect("fake observation write");
}
