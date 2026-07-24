//! Offline stdio MCP fake for Feature 007 acceptance.

use std::{
    env,
    fs::OpenOptions,
    io::{BufRead, Write},
    path::PathBuf,
};

use serde_json::{Value, json};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mode = std::fs::read_to_string(".workbench-fake-mcp-mode")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| env::var("WORKBENCH_FAKE_MCP_MODE").ok())
        .unwrap_or_else(|| "happy".to_owned());

    if args.first().is_some_and(|argument| argument == "--version") {
        println!("fake-mcp/1.0.0");
        observe("version");
        return;
    }

    run_stdio(&mode);
}

fn run_stdio(mode: &str) {
    observe("spawn");
    eprintln!("STDERR-MARKER-F007");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    while let Some(Ok(line)) = lines.next() {
        if line.len() > 8 * 1024 * 1024 {
            std::process::exit(70);
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => std::process::exit(65),
        };
        let id = request.get("id").cloned().unwrap_or(json!(1));
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        match (mode, method) {
            ("hang", "tools/call") => {
                observe("hang");
                loop {
                    std::thread::sleep(std::time::Duration::from_mins(1));
                }
            }
            ("crash-after-start", "tools/call") => {
                observe("crash");
                std::process::exit(99);
            }
            ("deny", "tools/call") => {
                observe("deny");
                writeln!(
                    stdout,
                    "{}",
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": "denied" }
                    })
                )
                .expect("write");
            }
            ("oversized", "tools/call") => {
                observe("oversized");
                let huge = "X".repeat(8 * 1024 * 1024 + 1);
                writeln!(
                    stdout,
                    "{}",
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": huge }
                    })
                )
                .expect("write");
            }
            (_, "tools/call") => {
                observe("call");
                let name = request["params"]["name"].as_str().unwrap_or("unknown");
                writeln!(
                    stdout,
                    "{}",
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": format!("ok:{name}") }],
                            "isError": false,
                            "privateSecret": "SECRET-MARKER-F007"
                        }
                    })
                )
                .expect("write");
            }
            _ => {
                writeln!(
                    stdout,
                    "{}",
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    })
                )
                .expect("write");
            }
        }
        let _ = stdout.flush();
    }
    observe("eof");
}

fn observe(event: &str) {
    let path = PathBuf::from(".workbench-fake-mcp-observation");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
}
