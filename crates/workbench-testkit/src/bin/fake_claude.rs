use std::{
    env,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::PathBuf,
};

use serde_json::{Value, json};

const VERSION: &str = "2.1.218";

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mode = std::fs::read_to_string(".workbench-fake-claude-mode")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| env::var("WORKBENCH_FAKE_CLAUDE_MODE").ok())
        .unwrap_or_else(|| "happy".to_owned());
    if args.first().is_some_and(|argument| argument == "--version") {
        println!("{VERSION} (Claude Code)");
        observe("version");
        return;
    }
    if args.first().is_some_and(|argument| argument == "auth") {
        emit_auth(&mode);
        observe("auth");
        return;
    }
    if args.iter().any(|argument| argument == "--print") {
        run_stream(&args, &mode);
        return;
    }
    std::process::exit(64);
}

fn emit_auth(mode: &str) {
    let status = match mode {
        "auth-not-logged" => json!({
            "loggedIn": false,
            "authMethod": "none",
            "apiProvider": "firstParty"
        }),
        "auth-api" => json!({
            "loggedIn": true,
            "authMethod": "apiKey",
            "apiProvider": "firstParty"
        }),
        "auth-alternate" => json!({
            "loggedIn": true,
            "authMethod": "claude.ai",
            "apiProvider": "bedrock"
        }),
        _ => json!({
            "loggedIn": true,
            "authMethod": "claude.ai",
            "apiProvider": "firstParty",
            "privateAuthMarker": "AUTH-MARKER-F005"
        }),
    };
    println!("{status}");
}

fn run_stream(args: &[String], mode: &str) {
    observe_launch(args);
    eprintln!("STDERR-MARKER-F005");
    let prompt_child = args.iter().any(|argument| argument == "--model");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut prompt_received = false;
    while let Some(Ok(line)) = lines.next() {
        let request: Value = serde_json::from_str(&line).expect("fake request JSON");
        match request.get("type").and_then(Value::as_str) {
            Some("control_request")
                if request["request"]["subtype"].as_str() == Some("initialize") =>
            {
                initialize(&mut stdout, &request, mode, prompt_child);
            }
            Some("user") => {
                prompt_received = true;
                observe("prompt");
                prompt(&mut stdout, mode);
                if mode == "malformed-truncated" {
                    return;
                }
            }
            Some("control_request")
                if request["request"]["subtype"].as_str() == Some("interrupt") =>
            {
                interrupt(&mut stdout, &request, mode, prompt_received);
            }
            _ => std::process::exit(65),
        }
    }
    observe("eof");
}

fn initialize(stdout: &mut impl Write, request: &Value, mode: &str, prompt_child: bool) {
    observe("initialize");
    if mode == "init-hang" || (mode == "prompt-init-hang" && prompt_child) {
        return;
    }
    let request_id = request["request_id"].clone();
    if mode == "init-interleaved" {
        emit(
            stdout,
            &json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "unrelated",
                    "response": {}
                }
            }),
        );
    }
    emit(
        stdout,
        &json!({
            "type": "control_response",
            "response": {
                "subtype": if mode == "init-error" { "error" } else { "success" },
                "request_id": request_id,
                "response": {"commands": []}
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "system",
            "subtype": "init",
            "session_id": "SESSION-MARKER-F005",
            "claude_code_version": if mode == "init-version-mismatch" {
                "2.1.217"
            } else {
                VERSION
            },
            "tools": ["Read", "Glob", "Grep"]
        }),
    );
}

fn prompt(stdout: &mut impl Write, mode: &str) {
    match mode {
        "crash" => std::process::exit(75),
        "malformed-duplicate" => emit_raw(stdout, r#"{"type":"result","type":"result"}"#),
        "malformed-utf8" => {
            stdout.write_all(&[0xff, b'\n']).expect("invalid UTF-8");
            stdout.flush().expect("flush invalid UTF-8");
        }
        "malformed-truncated" => {
            stdout
                .write_all(b"{\"type\":\"result\"")
                .expect("truncated frame");
            stdout.flush().expect("flush truncated frame");
        }
        "malformed-empty" => emit_raw(stdout, ""),
        "malformed-envelope" => emit(stdout, &json!({"type": "future_authority"})),
        "denied-tool" => emit(
            stdout,
            &json!({
                "type": "assistant",
                "session_id": "SESSION-MARKER-F005",
                "message": {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "name": "Bash",
                        "input": {"privateToolMarker": "TOOL-MARKER-F005"}
                    }]
                }
            }),
        ),
        "wait-cancel"
        | "cancel-ack-only"
        | "cancel-result-before-ack"
        | "cancel-error-result"
        | "cancel-silence"
        | "cancel-eof"
        | "cancel-crash" => {}
        _ => emit_happy(stdout),
    }
}

fn interrupt(stdout: &mut impl Write, request: &Value, mode: &str, prompt_received: bool) {
    observe(if prompt_received {
        "interrupt"
    } else {
        "preflight_interrupt"
    });
    if !prompt_received {
        emit(
            stdout,
            &json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request["request_id"],
                    "response": {}
                }
            }),
        );
        return;
    }
    match mode {
        "cancel-silence" => {}
        "cancel-eof" => std::process::exit(0),
        "cancel-crash" => std::process::exit(76),
        _ => {
            if mode == "cancel-result-before-ack" {
                emit(
                    stdout,
                    &json!({
                        "type": "result",
                        "subtype": "success",
                        "is_error": false,
                        "session_id": "SESSION-MARKER-F005",
                        "terminal_reason": "aborted_tools"
                    }),
                );
            }
            emit(
                stdout,
                &json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": request["request_id"],
                        "response": {}
                    }
                }),
            );
            match mode {
                "wait-cancel" => emit(
                    stdout,
                    &json!({
                        "type": "result",
                        "subtype": "success",
                        "is_error": false,
                        "session_id": "SESSION-MARKER-F005",
                        "terminal_reason": "aborted_tools"
                    }),
                ),
                "cancel-error-result" => emit(
                    stdout,
                    &json!({
                        "type": "result",
                        "subtype": "error_during_execution",
                        "is_error": true,
                        "session_id": "SESSION-MARKER-F005",
                        "terminal_reason": "error"
                    }),
                ),
                _ => {}
            }
        }
    }
}

fn emit_happy(stdout: &mut impl Write) {
    emit(
        stdout,
        &json!({
            "type": "stream_event",
            "uuid": "text-event",
            "session_id": "SESSION-MARKER-F005",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "deterministic Claude response"}
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "stream_event",
            "uuid": "thinking-event",
            "session_id": "SESSION-MARKER-F005",
            "event": {
                "type": "content_block_delta",
                "delta": {
                    "type": "thinking_delta",
                    "thinking": "THINKING-MARKER-F005"
                }
            }
        }),
    );
    emit(
        stdout,
        &json!({
            "type": "assistant",
            "session_id": "SESSION-MARKER-F005",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "deterministic Claude response"},
                    {
                        "type": "tool_use",
                        "name": "Read",
                        "input": {"file_path": "TOOL-MARKER-F005"}
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
            "session_id": "SESSION-MARKER-F005",
            "terminal_reason": "completed",
            "usage": {"privateUsageMarker": "USAGE-MARKER-F005"}
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
            "--output-format"
                | "--input-format"
                | "--permission-mode"
                | "--tools"
                | "--allowedTools"
                | "--mcp-config"
        ) {
            observe(&format!("{}={}", window[0], window[1]));
        }
    }
    for flag in [
        "--safe-mode",
        "--disable-slash-commands",
        "--no-chrome",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--setting-sources=",
    ] {
        if args.iter().any(|argument| argument == flag) {
            observe(flag);
        }
    }
    observe(&format!(
        "autoupdater={}",
        env::var("DISABLE_AUTOUPDATER").unwrap_or_default()
    ));
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        observe(&format!("{name}_present={}", env::var_os(name).is_some()));
    }
}

fn observe(event: &str) {
    let path = env::var_os("WORKBENCH_FAKE_CLAUDE_OBSERVATION").map_or_else(
        || PathBuf::from(".workbench-fake-claude-observation"),
        PathBuf::from,
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("fake observation");
    writeln!(file, "{event}").expect("fake observation write");
}
