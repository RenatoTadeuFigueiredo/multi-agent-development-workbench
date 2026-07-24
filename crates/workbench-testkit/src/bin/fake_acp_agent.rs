//! Deterministic offline ACP subprocess used by integration and acceptance tests.

#![forbid(unsafe_code)]

use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    process,
    str::FromStr,
};

use serde_json::{Value, json};

const MAX_FRAME_BYTES: usize = 8_388_608;
const MODE_ENV: &str = "WORKBENCH_FAKE_ACP_MODE";
const OBSERVATION_ENV: &str = "WORKBENCH_FAKE_ACP_OBSERVATION";
const UPDATE_DISABLED_ENV: &str = "GROK_DISABLE_AUTOUPDATER";
const MODE_FILE: &str = ".workbench-fake-acp-mode";
const OBSERVATION_FILE: &str = ".workbench-fake-acp-observation.ndjson";
const SESSION_ID: &str = "fake-acp-session";
const PERMISSION_REQUEST_ID: &str = "fake-permission-1";
const AUTH_SECRET: &str = "AUTH-MARKER-F004";
const SESSION_SECRET: &str = "SESSION-MARKER-F004";
const STDERR_SECRET: &str = "STDERR-MARKER-F004";
const ERROR_SECRET: &str = "ERROR-MARKER-F004";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Happy,
    Authenticate,
    ReversePermission,
    CancelConfirmed,
    CancelUnconfirmed,
    CrashInitialize,
    CrashPrompt,
    Malformed,
    DuplicateKeys,
    InvalidUtf8,
    Truncated,
    InvalidJsonRpc,
    EmptyFrame,
    ExactLimit,
    Oversize,
    Hang,
    CancelEof,
    CancelExit,
    CancelError,
    CancelEndTurn,
    SecretError,
    CompatibleUpdate,
    IncompatibleVersion,
    MissingCapability,
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "happy" | "stream" => Ok(Self::Happy),
            "authenticate" => Ok(Self::Authenticate),
            "reverse-permission" => Ok(Self::ReversePermission),
            "cancel-confirmed" => Ok(Self::CancelConfirmed),
            "cancel-unconfirmed" => Ok(Self::CancelUnconfirmed),
            "crash-initialize" => Ok(Self::CrashInitialize),
            "crash-prompt" => Ok(Self::CrashPrompt),
            "malformed" => Ok(Self::Malformed),
            "duplicate-keys" => Ok(Self::DuplicateKeys),
            "invalid-utf8" => Ok(Self::InvalidUtf8),
            "truncated" => Ok(Self::Truncated),
            "invalid-jsonrpc" => Ok(Self::InvalidJsonRpc),
            "empty-frame" => Ok(Self::EmptyFrame),
            "exact-limit" => Ok(Self::ExactLimit),
            "oversize" => Ok(Self::Oversize),
            "hang" => Ok(Self::Hang),
            "cancel-eof" => Ok(Self::CancelEof),
            "cancel-exit" => Ok(Self::CancelExit),
            "cancel-error" => Ok(Self::CancelError),
            "cancel-end-turn" => Ok(Self::CancelEndTurn),
            "secret-error" => Ok(Self::SecretError),
            "compatible-update" => Ok(Self::CompatibleUpdate),
            "incompatible-version" => Ok(Self::IncompatibleVersion),
            "missing-capability" => Ok(Self::MissingCapability),
            _ => Err(format!("unknown fake ACP mode: {value}")),
        }
    }
}

#[derive(Debug)]
struct PendingPrompt {
    id: Value,
    session_id: String,
}

struct FakeAgent {
    mode: Mode,
    observation: Option<File>,
    pending_prompt: Option<PendingPrompt>,
}

impl FakeAgent {
    fn new(mode: Mode, observation: Option<File>) -> Self {
        Self {
            mode,
            observation,
            pending_prompt: None,
        }
    }

    fn record(&mut self, event: &Value) -> io::Result<()> {
        let Some(file) = &mut self.observation else {
            return Ok(());
        };
        serde_json::to_writer(&mut *file, &event)?;
        file.write_all(b"\n")?;
        file.flush()
    }

    fn handle(
        &mut self,
        request: &Value,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<LoopControl, String> {
        let object = request
            .as_object()
            .ok_or_else(|| "JSON-RPC envelope must be an object".to_owned())?;
        if object.get("jsonrpc") != Some(&Value::String("2.0".to_owned())) {
            return Err("JSON-RPC version must be 2.0".to_owned());
        }

        if let Some(method) = object.get("method").and_then(Value::as_str) {
            self.record(&json!({
                "event": "request",
                "method": method,
                "has_id": object.contains_key("id")
            }))
            .map_err(|error| error.to_string())?;
            return self.handle_method(method, object, output);
        }

        if object.get("id") == Some(&Value::String(PERMISSION_REQUEST_ID.to_owned())) {
            self.record(&json!({
                "event": "permission_response",
                "denied": permission_was_denied(object.get("result"))
            }))
            .map_err(|error| error.to_string())?;
            if let Some(prompt) = self.pending_prompt.take() {
                self.finish_prompt(&prompt, output)?;
            }
            return Ok(LoopControl::Continue);
        }

        Err("unexpected JSON-RPC response".to_owned())
    }

    fn handle_method(
        &mut self,
        method: &str,
        object: &serde_json::Map<String, Value>,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<LoopControl, String> {
        match method {
            "initialize" => self.initialize(object, output),
            "authenticate" => {
                write_result(output, request_id(object)?, &json!({}))?;
                Ok(LoopControl::Continue)
            }
            "session/new" => {
                let session_id = if self.mode == Mode::SecretError {
                    SESSION_SECRET
                } else {
                    SESSION_ID
                };
                write_result(
                    output,
                    request_id(object)?,
                    &json!({
                        "sessionId": session_id,
                        "modes": {"availableModes": []},
                        "models": {"availableModels": []}
                    }),
                )?;
                Ok(LoopControl::Continue)
            }
            "session/load" => {
                write_result(
                    output,
                    request_id(object)?,
                    &json!({"sessionId": session_id(object).unwrap_or(SESSION_ID)}),
                )?;
                Ok(LoopControl::Continue)
            }
            "session/prompt" => self.prompt(object, output),
            "session/cancel" => self.cancel(output),
            _ => {
                if let Some(id) = object.get("id") {
                    write_json(
                        output,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32601,
                                "message": "method not supported by fake ACP agent"
                            }
                        }),
                    )?;
                }
                Ok(LoopControl::Continue)
            }
        }
    }

    fn initialize(
        &mut self,
        object: &serde_json::Map<String, Value>,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<LoopControl, String> {
        if let Some(control) = self.initialize_fault_fixture(object, output)? {
            return Ok(control);
        }

        let protocol_version = if self.mode == Mode::IncompatibleVersion {
            2
        } else {
            1
        };
        let load_session = self.mode != Mode::MissingCapability;
        let auth_methods = if self.mode == Mode::Authenticate {
            json!([{
                "id": "fake-subscription",
                "name": "Fake subscription",
                "description": "Deterministic offline authentication"
            }])
        } else if self.mode == Mode::SecretError {
            json!([{
                "id": AUTH_SECRET,
                "name": "Secret fixture",
                "description": "Secret fixture"
            }])
        } else {
            json!([])
        };
        let version = if self.mode == Mode::CompatibleUpdate {
            "1.1.0-test"
        } else {
            "1.0.0-test"
        };
        let mut result = json!({
            "protocolVersion": protocol_version,
            "agentCapabilities": {
                "loadSession": load_session
            },
            "agentInfo": {
                "name": "fake-grok",
                "title": "Fake Grok ACP Agent",
                "version": version
            },
            "authMethods": auth_methods
        });
        if self.mode == Mode::SecretError {
            result["_meta"] = json!({"defaultAuthMethodId": AUTH_SECRET});
        }
        if self.mode == Mode::CompatibleUpdate {
            result["_meta"] = json!({"compatibleFixture": true});
            result["additiveCapability"] = json!({"ignored": true});
        }
        write_result(output, request_id(object)?, &result)?;
        Ok(LoopControl::Continue)
    }

    fn initialize_fault_fixture(
        &mut self,
        object: &serde_json::Map<String, Value>,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<Option<LoopControl>, String> {
        if self.mode == Mode::CrashInitialize {
            self.record(&json!({"event": "crash", "phase": "initialize"}))
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Exit(71)));
        }
        if self.mode == Mode::Malformed {
            output
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\n")
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::DuplicateKeys {
            output
                .write_all(
                    b"{\"jsonrpc\":\"2.0\",\"jsonrpc\":\"2.0\",\"id\":\"duplicate\",\"result\":{}}\n",
                )
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::InvalidUtf8 {
            output
                .write_all(&[0xff, b'\n'])
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::Truncated {
            output
                .write_all(b"{\"jsonrpc\":\"2.0\"")
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Exit(0)));
        }
        if self.mode == Mode::InvalidJsonRpc {
            write_json(
                output,
                &json!({
                    "jsonrpc": "1.0",
                    "id": request_id(object)?,
                    "result": {
                        "protocolVersion": 1,
                        "agentCapabilities": {"loadSession": true},
                        "authMethods": []
                    }
                }),
            )?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::EmptyFrame {
            output
                .write_all(b"\n")
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::ExactLimit {
            write_exact_limit_initialize(output, request_id(object)?)?;
            return Ok(Some(LoopControl::Continue));
        }
        if self.mode == Mode::Oversize {
            let bytes = vec![b'x'; MAX_FRAME_BYTES + 1];
            output
                .write_all(&bytes)
                .and_then(|()| output.write_all(b"\n"))
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
            return Ok(Some(LoopControl::Continue));
        }
        Ok(None)
    }

    fn prompt(
        &mut self,
        object: &serde_json::Map<String, Value>,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<LoopControl, String> {
        if self.mode == Mode::CrashPrompt {
            self.record(&json!({"event": "crash", "phase": "prompt"}))
                .map_err(|error| error.to_string())?;
            return Ok(LoopControl::Exit(72));
        }

        let pending = PendingPrompt {
            id: request_id(object)?.clone(),
            session_id: session_id(object).unwrap_or(SESSION_ID).to_owned(),
        };
        self.emit_update(&pending.session_id, output)?;

        match self.mode {
            Mode::CancelConfirmed
            | Mode::CancelUnconfirmed
            | Mode::Hang
            | Mode::CancelEof
            | Mode::CancelExit
            | Mode::CancelError
            | Mode::CancelEndTurn => {
                self.pending_prompt = Some(pending);
            }
            Mode::SecretError => {
                write_error(output, &pending.id, ERROR_SECRET)?;
            }
            Mode::ReversePermission => {
                self.pending_prompt = Some(pending);
                write_json(
                    output,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": PERMISSION_REQUEST_ID,
                        "method": "session/request_permission",
                        "params": {
                            "sessionId": SESSION_ID,
                            "toolCall": {
                                "toolCallId": "fake-tool-call",
                                "title": "Fake protected operation",
                                "kind": "execute",
                                "status": "pending"
                            },
                            "options": [{
                                "optionId": "reject",
                                "name": "Reject",
                                "kind": "reject_once"
                            }]
                        }
                    }),
                )?;
            }
            _ => self.finish_prompt(&pending, output)?,
        }
        Ok(LoopControl::Continue)
    }

    fn emit_update(
        &mut self,
        session_id: &str,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<(), String> {
        write_json(
            output,
            &json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {
                            "type": "text",
                            "text": "deterministic ACP output"
                        }
                    }
                }
            }),
        )?;
        if self.mode == Mode::CompatibleUpdate {
            write_json(
                output,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "fake/additive_notification",
                    "params": {"ignored": true}
                }),
            )?;
        }
        Ok(())
    }

    fn finish_prompt(
        &mut self,
        prompt: &PendingPrompt,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<(), String> {
        write_result(output, &prompt.id, &json!({"stopReason": "end_turn"}))?;
        self.record(&json!({"event": "prompt_finished", "stop_reason": "end_turn"}))
            .map_err(|error| error.to_string())
    }

    fn cancel(
        &mut self,
        output: &mut BufWriter<io::StdoutLock<'_>>,
    ) -> Result<LoopControl, String> {
        self.record(&json!({"event": "cancel_received"}))
            .map_err(|error| error.to_string())?;
        if self.mode == Mode::CancelConfirmed
            && let Some(prompt) = self.pending_prompt.take()
        {
            write_result(output, &prompt.id, &json!({"stopReason": "cancelled"}))?;
            self.record(&json!({
                "event": "prompt_finished",
                "stop_reason": "cancelled"
            }))
            .map_err(|error| error.to_string())?;
        }
        if self.mode == Mode::CancelEof {
            return Ok(LoopControl::Exit(0));
        }
        if self.mode == Mode::CancelExit {
            return Ok(LoopControl::Exit(72));
        }
        if self.mode == Mode::CancelError
            && let Some(prompt) = self.pending_prompt.take()
        {
            write_error(output, &prompt.id, "ambiguous cancellation error")?;
        }
        if self.mode == Mode::CancelEndTurn
            && let Some(prompt) = self.pending_prompt.take()
        {
            write_result(output, &prompt.id, &json!({"stopReason": "end_turn"}))?;
        }
        Ok(LoopControl::Continue)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoopControl {
    Continue,
    Exit(i32),
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("fake ACP agent failed: {error}");
            process::exit(70);
        }
    }
}

fn run() -> Result<i32, String> {
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let workspace_mode = std::fs::read_to_string(cwd.join(MODE_FILE))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let selected_mode = env::var(MODE_ENV).ok().or(workspace_mode);
    let mode = selected_mode
        .as_deref()
        .map_or(Ok(Mode::Happy), str::parse)?;
    if env::args().skip(1).eq(["--version"]) {
        let version = if mode == Mode::CompatibleUpdate {
            "1.1.0-test"
        } else {
            "1.0.0-test"
        };
        println!("grok {version} [stable]");
        return Ok(0);
    }
    let (profile, mode_override) = parse_arguments()?;
    if profile != ["agent", "--no-leader", "stdio"] {
        return Err("expected launch profile: agent --no-leader stdio".to_owned());
    }
    let mode = mode_override.map_or(Ok(mode), |value| value.parse())?;
    let observation = observation_file(&cwd)?;
    let mut agent = FakeAgent::new(mode, observation);
    agent
        .record(&json!({
            "event": "started",
            "mode": format!("{mode:?}"),
            "argv": profile,
            "cwd": cwd,
            "autoupdater": env::var(UPDATE_DISABLED_ENV).ok(),
            "pid": process::id()
        }))
        .map_err(|error| error.to_string())?;
    if mode == Mode::SecretError {
        eprintln!("{STDERR_SECRET}");
        agent
            .record(&json!({
                "event": "secret_fixture",
                "markers": [AUTH_SECRET, SESSION_SECRET, STDERR_SECRET, ERROR_SECRET]
            }))
            .map_err(|error| error.to_string())?;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    loop {
        let Some(frame) = read_frame(&mut input).map_err(|error| error.to_string())? else {
            agent
                .record(&json!({"event": "eof"}))
                .map_err(|error| error.to_string())?;
            return Ok(0);
        };
        let request =
            serde_json::from_slice(&frame).map_err(|_| "invalid JSON-RPC request".to_owned())?;
        match agent.handle(&request, &mut output)? {
            LoopControl::Continue => {}
            LoopControl::Exit(code) => return Ok(code),
        }
    }
}

fn parse_arguments() -> Result<(Vec<String>, Option<String>), String> {
    let mut profile = Vec::new();
    let mut mode = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--mode" {
            mode = Some(
                arguments
                    .next()
                    .ok_or_else(|| "--mode requires a value".to_owned())?,
            );
        } else {
            profile.push(argument);
        }
    }
    Ok((profile, mode))
}

fn observation_file(workspace: &std::path::Path) -> Result<Option<File>, String> {
    let path = env::var_os(OBSERVATION_ENV).map(PathBuf::from).or_else(|| {
        workspace
            .join(MODE_FILE)
            .is_file()
            .then(|| workspace.join(OBSERVATION_FILE))
    });
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.is_absolute() {
        return Err("fake ACP observation path must be absolute".to_owned());
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn request_id(object: &serde_json::Map<String, Value>) -> Result<&Value, String> {
    object
        .get("id")
        .ok_or_else(|| "JSON-RPC request requires an id".to_owned())
}

fn session_id(object: &serde_json::Map<String, Value>) -> Option<&str> {
    object
        .get("params")
        .and_then(Value::as_object)
        .and_then(|params| params.get("sessionId"))
        .and_then(Value::as_str)
}

fn write_result(
    output: &mut BufWriter<io::StdoutLock<'_>>,
    id: &Value,
    result: &Value,
) -> Result<(), String> {
    write_json(
        output,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
}

fn write_error(
    output: &mut BufWriter<io::StdoutLock<'_>>,
    id: &Value,
    message: &str,
) -> Result<(), String> {
    write_json(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": message}
        }),
    )
}

fn write_exact_limit_initialize(
    output: &mut BufWriter<io::StdoutLock<'_>>,
    id: &Value,
) -> Result<(), String> {
    let mut response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": 1,
            "agentCapabilities": {"loadSession": true},
            "agentInfo": {
                "name": "fake-grok",
                "version": "1.0.0-test"
            },
            "authMethods": [],
            "_meta": {"padding": ""}
        }
    });
    let baseline = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    let padding = MAX_FRAME_BYTES
        .checked_sub(baseline.len())
        .ok_or_else(|| "exact-limit fixture baseline is too large".to_owned())?;
    response["result"]["_meta"]["padding"] = Value::String("a".repeat(padding));
    let encoded = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    if encoded.len() != MAX_FRAME_BYTES {
        return Err("exact-limit fixture has the wrong size".to_owned());
    }
    output
        .write_all(&encoded)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|error| error.to_string())
}

fn write_json(output: &mut BufWriter<io::StdoutLock<'_>>, value: &Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err("fake ACP output frame exceeds 8 MiB".to_owned());
    }
    output
        .write_all(&encoded)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|error| error.to_string())
}

fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete fake ACP frame",
                ))
            };
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if frame.len() + newline > MAX_FRAME_BYTES {
                reader.consume(newline + 1);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fake ACP input frame exceeds 8 MiB",
                ));
            }
            frame.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            if frame.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "empty fake ACP frame",
                ));
            }
            return Ok(Some(frame));
        }
        if frame.len() + available.len() > MAX_FRAME_BYTES {
            let consumed = available.len();
            reader.consume(consumed);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fake ACP input frame exceeds 8 MiB",
            ));
        }
        frame.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }
}

fn permission_was_denied(result: Option<&Value>) -> bool {
    result
        .and_then(|value| value.get("outcome"))
        .and_then(|value| value.get("outcome"))
        .and_then(Value::as_str)
        .is_some_and(|outcome| outcome == "cancelled")
}
