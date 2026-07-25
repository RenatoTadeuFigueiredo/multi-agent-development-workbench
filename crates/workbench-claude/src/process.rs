use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use serde::Deserialize;
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStderr, ChildStdin, Command},
    task::JoinHandle,
};

use crate::{
    ClaudeError, ClaudeErrorKind,
    codec::{FrameReader, write_frame},
    protocol::{Inbound, initialize_request, parse_inbound_with_policy},
};

const DEFAULT_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_millis(100);
const AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_OUTPUT_BYTES: usize = 4_096;
const MAX_AUTH_READ_BYTES: u64 = 4_097;
const MAX_VERSION_BYTES: usize = 256;

#[derive(Clone, Debug)]
pub struct ClaudeLaunchProfile {
    executable: PathBuf,
    workspace: PathBuf,
    initialization_timeout: Duration,
    shutdown_grace: Duration,
    /// When true, provider-native Write/Edit tools are launched under policy.
    native_writes: bool,
}

impl ClaudeLaunchProfile {
    pub fn new(executable: impl Into<PathBuf>, workspace: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            workspace: workspace.into(),
            initialization_timeout: DEFAULT_INITIALIZATION_TIMEOUT,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            native_writes: false,
        }
    }

    #[must_use]
    pub const fn initialization_timeout(mut self, timeout: Duration) -> Self {
        self.initialization_timeout = timeout;
        self
    }

    #[must_use]
    pub const fn shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }

    /// Enables provider-native Write/Edit tools (fail-closed default is off).
    #[must_use]
    pub const fn native_writes(mut self, enabled: bool) -> Self {
        self.native_writes = enabled;
        self
    }

    #[must_use]
    pub const fn native_writes_enabled(&self) -> bool {
        self.native_writes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub reaped: bool,
    pub forced: bool,
}

pub(crate) struct ClaudeProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: FrameReader<tokio::process::ChildStdout>,
    stderr: JoinHandle<()>,
    shutdown_grace: Duration,
    native_writes: bool,
}

impl ClaudeProcess {
    pub(crate) async fn spawn_initialized(
        profile: &ClaudeLaunchProfile,
        expected_version: &str,
        model: Option<&str>,
        session_id: &str,
    ) -> Result<Self, ClaudeError> {
        validate_profile(profile, expected_version, model, session_id)?;
        let executable = canonical_executable(&profile.executable)?;
        let workspace = canonical_workspace(&profile.workspace)?;
        let mut command = Command::new(executable);
        command
            .args([
                "--print",
                "--output-format",
                "stream-json",
                "--verbose",
                "--input-format",
                "stream-json",
                "--include-partial-messages",
                "--permission-mode",
                if profile.native_writes {
                    "default"
                } else {
                    "dontAsk"
                },
                "--tools",
                if profile.native_writes {
                    "Read,Glob,Grep,Write,Edit"
                } else {
                    "Read,Glob,Grep"
                },
                "--allowedTools",
                if profile.native_writes {
                    "Read,Glob,Grep,Write,Edit"
                } else {
                    "Read,Glob,Grep"
                },
                "--safe-mode",
                "--disable-slash-commands",
                "--no-chrome",
                "--no-session-persistence",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--setting-sources=",
            ])
            .arg(format!("--session-id={session_id}"))
            .env("DISABLE_AUTOUPDATER", "1")
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let Some(model) = model {
            command.args(["--model", model]);
        }
        sanitize_environment(&mut command);
        let mut child = command.spawn().map_err(|_| spawn_failed())?;
        let Some(stdin) = child.stdin.take() else {
            return Err(spawn_failed());
        };
        let Some(stdout) = child.stdout.take() else {
            return Err(spawn_failed());
        };
        let Some(stderr) = child.stderr.take() else {
            return Err(spawn_failed());
        };
        let mut process = Self {
            child,
            stdin: Some(stdin),
            reader: FrameReader::new(stdout),
            stderr: spawn_stderr_drain(stderr),
            shutdown_grace: profile.shutdown_grace,
            native_writes: profile.native_writes,
        };
        let initialized = process.initialize(expected_version);
        match tokio::time::timeout(profile.initialization_timeout, initialized).await {
            Ok(Ok(())) => Ok(process),
            Ok(Err(error)) => {
                let report = process.shutdown().await;
                if report.reaped {
                    Err(error)
                } else {
                    Err(reap_failed())
                }
            }
            Err(_) => {
                let report = process.shutdown().await;
                if report.reaped {
                    Err(ClaudeError::new(
                        ClaudeErrorKind::Timeout,
                        "Claude Code initialization timed out",
                    ))
                } else {
                    Err(reap_failed())
                }
            }
        }
    }

    async fn initialize(&mut self, expected_version: &str) -> Result<(), ClaudeError> {
        let request_id = format!("initialize-{}", uuid::Uuid::now_v7());
        self.write(&initialize_request(&request_id)).await?;
        let mut response_received = false;
        let mut init_received = false;
        while !response_received || !init_received {
            let value = self
                .reader
                .next_frame()
                .await?
                .ok_or_else(transport_closed)?;
            match parse_inbound_with_policy(&value, self.native_writes)? {
                Inbound::ControlResponse {
                    request_id: response_id,
                    success,
                } if response_id == request_id => {
                    if !success {
                        return Err(incompatible());
                    }
                    response_received = true;
                }
                Inbound::ControlResponse { .. } | Inbound::Ignored => {}
                Inbound::SystemInit { version } => {
                    if version
                        .as_deref()
                        .is_some_and(|reported| reported != expected_version)
                    {
                        return Err(incompatible());
                    }
                    init_received = true;
                }
                Inbound::TextDelta(_)
                | Inbound::Assistant { .. }
                | Inbound::ToolStarted(_)
                | Inbound::Result { .. } => return Err(incompatible()),
            }
        }
        Ok(())
    }

    pub(crate) async fn write(&mut self, value: &serde_json::Value) -> Result<(), ClaudeError> {
        let stdin = self.stdin.as_mut().ok_or_else(transport_closed)?;
        write_frame(stdin, value).await
    }

    pub(crate) async fn next(&mut self) -> Result<Option<Inbound>, ClaudeError> {
        let native_writes = self.native_writes;
        self.reader
            .next_frame()
            .await?
            .as_ref()
            .map(|value| parse_inbound_with_policy(value, native_writes))
            .transpose()
    }

    async fn verify_interrupt_receipt(&mut self, timeout: Duration) -> Result<(), ClaudeError> {
        let request_id = format!("preflight-interrupt-{}", uuid::Uuid::now_v7());
        self.write(&crate::protocol::interrupt_request(&request_id))
            .await?;
        let receipt = async {
            loop {
                let Some(inbound) = self.next().await? else {
                    return Err(transport_closed());
                };
                match inbound {
                    Inbound::ControlResponse {
                        request_id: response_id,
                        success,
                    } if response_id == request_id => {
                        return if success { Ok(()) } else { Err(incompatible()) };
                    }
                    Inbound::ControlResponse { .. }
                    | Inbound::SystemInit { .. }
                    | Inbound::Ignored => {}
                    Inbound::TextDelta(_)
                    | Inbound::Assistant { .. }
                    | Inbound::ToolStarted(_)
                    | Inbound::Result { .. } => return Err(incompatible()),
                }
            }
        };
        tokio::time::timeout(timeout, receipt).await.map_err(|_| {
            ClaudeError::new(
                ClaudeErrorKind::Timeout,
                "Claude Code interrupt preflight timed out",
            )
        })?
    }

    pub(crate) async fn shutdown(mut self) -> ShutdownReport {
        let process_group = child_process_group(&self.child);
        self.stdin.take();
        let mut forced = false;
        let child_reaped = if wait_for_exit(&mut self.child, self.shutdown_grace).await {
            true
        } else {
            signal_process_group(process_group, Signal::TERM);
            if wait_for_exit(&mut self.child, self.shutdown_grace).await {
                true
            } else {
                forced = true;
                signal_process_group(process_group, Signal::KILL);
                let _ignored = self.child.start_kill();
                wait_for_exit(&mut self.child, self.shutdown_grace).await
            }
        };
        let group_reaped =
            reap_remaining_process_group(process_group, self.shutdown_grace, &mut forced).await;
        finish_stderr(self.stderr).await;
        ShutdownReport {
            reaped: child_reaped && group_reaped,
            forced,
        }
    }
}

async fn reap_remaining_process_group(
    process_group: Option<Pid>,
    grace: Duration,
    forced: &mut bool,
) -> bool {
    let Some(process_group) = process_group else {
        return false;
    };
    if test_kill_process_group(process_group).is_err() {
        return true;
    }
    let _ignored = kill_process_group(process_group, Signal::TERM);
    if wait_for_process_group_exit(process_group, grace).await {
        return true;
    }
    *forced = true;
    let _ignored = kill_process_group(process_group, Signal::KILL);
    wait_for_process_group_exit(process_group, grace).await
}

async fn wait_for_process_group_exit(process_group: Pid, timeout: Duration) -> bool {
    let exited = async {
        while test_kill_process_group(process_group).is_ok() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(timeout, exited).await.is_ok()
}

fn signal_process_group(process_group: Option<Pid>, signal: Signal) {
    if let Some(process_group) = process_group {
        let _ignored = kill_process_group(process_group, signal);
    }
}

fn child_process_group(child: &Child) -> Option<Pid> {
    child
        .id()
        .and_then(|raw_pid| Pid::from_raw(raw_pid.cast_signed()))
}

pub(crate) async fn preflight_subscription(
    profile: &ClaudeLaunchProfile,
    expected_version: &str,
) -> Result<(), ClaudeError> {
    if profile.initialization_timeout.is_zero() || profile.shutdown_grace.is_zero() {
        return Err(invalid_configuration());
    }
    probe_subscription_auth(profile).await?;
    let session_id = uuid::Uuid::now_v7().to_string();
    let mut process =
        ClaudeProcess::spawn_initialized(profile, expected_version, None, &session_id).await?;
    if let Err(error) = process
        .verify_interrupt_receipt(profile.initialization_timeout)
        .await
    {
        let report = process.shutdown().await;
        return if report.reaped {
            Err(error)
        } else {
            Err(reap_failed())
        };
    }
    let report = process.shutdown().await;
    if report.reaped {
        Ok(())
    } else {
        Err(reap_failed())
    }
}

async fn probe_subscription_auth(profile: &ClaudeLaunchProfile) -> Result<(), ClaudeError> {
    let executable = canonical_executable(&profile.executable)?;
    let workspace = canonical_workspace(&profile.workspace)?;
    let mut command = Command::new(executable);
    command
        .args(["auth", "status", "--json"])
        .env("DISABLE_AUTOUPDATER", "1")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_environment(&mut command);
    let mut child = command.spawn().map_err(|_| spawn_failed())?;
    let process_group = child_process_group(&child);
    let Some(stdout) = child.stdout.take() else {
        reap_probe(&mut child, process_group).await;
        return Err(spawn_failed());
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_AUTH_READ_BYTES)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    };
    let bytes = match tokio::time::timeout(AUTH_PROBE_TIMEOUT, read).await {
        Ok(Ok(bytes)) if bytes.len() <= MAX_AUTH_OUTPUT_BYTES => bytes,
        _ => {
            if reap_probe(&mut child, process_group).await {
                return Err(authentication_required());
            }
            return Err(reap_failed());
        }
    };
    let status = match tokio::time::timeout(AUTH_PROBE_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => status,
        _ => {
            if reap_probe(&mut child, process_group).await {
                return Err(authentication_required());
            }
            return Err(reap_failed());
        }
    };
    let _ = status;
    let mut forced = false;
    if !reap_remaining_process_group(process_group, DEFAULT_SHUTDOWN_GRACE, &mut forced).await {
        return Err(reap_failed());
    }
    validate_subscription_auth(&bytes)
}

fn validate_subscription_auth(bytes: &[u8]) -> Result<(), ClaudeError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthStatus {
        logged_in: bool,
        auth_method: String,
        api_provider: String,
    }

    let status: AuthStatus =
        serde_json::from_slice(bytes).map_err(|_| authentication_required())?;
    if status.logged_in
        && matches!(status.auth_method.as_str(), "claude.ai" | "claudeai")
        && status.api_provider == "firstParty"
        && status.auth_method.len() <= 64
        && status.api_provider.len() <= 64
    {
        Ok(())
    } else {
        Err(authentication_required())
    }
}

pub(crate) fn sanitize_environment(command: &mut Command) {
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "ANTHROPIC_BEDROCK_BASE_URL",
        "ANTHROPIC_VERTEX_BASE_URL",
        "ANTHROPIC_FOUNDRY_BASE_URL",
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_AGENT_SDK_VERSION",
    ] {
        command.env_remove(name);
    }
}

fn validate_profile(
    profile: &ClaudeLaunchProfile,
    expected_version: &str,
    model: Option<&str>,
    session_id: &str,
) -> Result<(), ClaudeError> {
    if profile.initialization_timeout.is_zero()
        || profile.shutdown_grace.is_zero()
        || expected_version.is_empty()
        || expected_version.len() > MAX_VERSION_BYTES
        || expected_version.chars().any(char::is_control)
        || session_id.is_empty()
        || session_id.len() > 64
        || session_id.chars().any(char::is_control)
        || model.is_some_and(|model| {
            model.is_empty()
                || model.len() > 4_096
                || model.starts_with('-')
                || model.chars().any(char::is_control)
        })
    {
        Err(invalid_configuration())
    } else {
        Ok(())
    }
}

fn canonical_executable(path: &Path) -> Result<PathBuf, ClaudeError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid_configuration())?;
    if !canonical.is_absolute() || !canonical.is_file() {
        return Err(invalid_configuration());
    }
    Ok(canonical)
}

fn canonical_workspace(path: &Path) -> Result<PathBuf, ClaudeError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid_configuration())?;
    if !canonical.is_absolute() || !canonical.is_dir() {
        return Err(invalid_configuration());
    }
    Ok(canonical)
}

fn spawn_stderr_drain(mut stderr: ChildStderr) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0_u8; 4_096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
}

async fn finish_stderr(task: JoinHandle<()>) {
    let mut task = task;
    if tokio::time::timeout(DEFAULT_SHUTDOWN_GRACE, &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ignored = task.await;
    }
}

async fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    matches!(tokio::time::timeout(timeout, child.wait()).await, Ok(Ok(_)))
}

async fn reap_probe(child: &mut Child, process_group: Option<Pid>) -> bool {
    signal_process_group(process_group, Signal::KILL);
    let _ignored = child.start_kill();
    let child_reaped = wait_for_exit(child, AUTH_PROBE_TIMEOUT).await;
    let mut forced = true;
    child_reaped
        && reap_remaining_process_group(process_group, DEFAULT_SHUTDOWN_GRACE, &mut forced).await
}

fn invalid_configuration() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::InvalidConfiguration,
        "Claude Code launch configuration is invalid",
    )
}

fn spawn_failed() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::SpawnFailed,
        "Claude Code process could not be started",
    )
}

fn authentication_required() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::AuthenticationRequired,
        "Claude Code subscription authentication is unavailable",
    )
}

fn incompatible() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::IncompatibleProtocol,
        "Claude Code protocol is incompatible",
    )
}

fn transport_closed() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::TransportClosed,
        "Claude Code transport is unavailable",
    )
}

fn reap_failed() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::ReapFailed,
        "Claude Code process could not be reaped",
    )
}
