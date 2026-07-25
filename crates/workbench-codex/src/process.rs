use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStderr, Command},
    task::JoinHandle,
};

use crate::{
    CodexError, CodexErrorKind,
    codec::FrameReader,
    protocol::{Inbound, parse_inbound_with_policy},
};

const DEFAULT_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_millis(100);
const AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_OUTPUT_BYTES: usize = 4_096;
const MAX_AUTH_READ_BYTES: u64 = 4_097;
const MAX_VERSION_BYTES: usize = 256;
const MAX_PROMPT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct CodexLaunchProfile {
    executable: PathBuf,
    workspace: PathBuf,
    preflight_timeout: Duration,
    shutdown_grace: Duration,
    /// When true, launches with workspace-write sandbox under central policy.
    native_writes: bool,
}

impl CodexLaunchProfile {
    pub fn new(executable: impl Into<PathBuf>, workspace: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            workspace: workspace.into(),
            preflight_timeout: DEFAULT_PREFLIGHT_TIMEOUT,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            native_writes: false,
        }
    }

    #[must_use]
    pub const fn preflight_timeout(mut self, timeout: Duration) -> Self {
        self.preflight_timeout = timeout;
        self
    }

    /// Enables workspace-write sandbox and file_change observation.
    #[must_use]
    pub const fn native_writes(mut self, enabled: bool) -> Self {
        self.native_writes = enabled;
        self
    }

    #[must_use]
    pub const fn native_writes_enabled(&self) -> bool {
        self.native_writes
    }

    #[must_use]
    pub const fn shutdown_grace(mut self, grace: Duration) -> Self {
        self.shutdown_grace = grace;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub reaped: bool,
    pub forced: bool,
}

pub(crate) struct CodexProcess {
    child: Child,
    reader: FrameReader<tokio::process::ChildStdout>,
    stderr: JoinHandle<()>,
    shutdown_grace: Duration,
    native_writes: bool,
}

impl CodexProcess {
    pub(crate) fn spawn_prompt(
        profile: &CodexLaunchProfile,
        model: &str,
        prompt: &str,
    ) -> Result<Self, CodexError> {
        validate_launch(profile, Some(model), Some(prompt))?;
        let executable = canonical_executable(&profile.executable)?;
        let workspace = canonical_workspace(&profile.workspace)?;
        let workspace_arg = workspace.to_string_lossy().into_owned();
        let mut command = Command::new(executable);
        command
            .args([
                "exec",
                "--json",
                "--ephemeral",
                "--sandbox",
                if profile.native_writes {
                    "workspace-write"
                } else {
                    "read-only"
                },
                "-C",
                workspace_arg.as_str(),
                "-m",
                model,
                prompt,
            ])
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        sanitize_environment(&mut command);
        let mut child = command.spawn().map_err(|_| spawn_failed())?;
        let Some(stdout) = child.stdout.take() else {
            return Err(spawn_failed());
        };
        let Some(stderr) = child.stderr.take() else {
            return Err(spawn_failed());
        };
        Ok(Self {
            child,
            reader: FrameReader::new(stdout),
            stderr: spawn_stderr_drain(stderr),
            shutdown_grace: profile.shutdown_grace,
            native_writes: profile.native_writes,
        })
    }

    pub(crate) async fn next(&mut self) -> Result<Option<Inbound>, CodexError> {
        let native_writes = self.native_writes;
        self.reader
            .next_frame()
            .await?
            .as_ref()
            .map(|value| parse_inbound_with_policy(value, native_writes))
            .transpose()
    }

    pub(crate) async fn shutdown(mut self) -> ShutdownReport {
        let process_group = child_process_group(&self.child);
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

/// Prompt-free `ChatGPT` subscription authentication and version identity check.
pub(crate) async fn preflight_subscription(
    profile: &CodexLaunchProfile,
    expected_version: &str,
) -> Result<(), CodexError> {
    if profile.preflight_timeout.is_zero() || profile.shutdown_grace.is_zero() {
        return Err(invalid_configuration());
    }
    let reported = probe_version(profile).await?;
    if reported != expected_version {
        return Err(incompatible());
    }
    probe_subscription_auth(profile).await
}

async fn probe_version(profile: &CodexLaunchProfile) -> Result<String, CodexError> {
    let executable = canonical_executable(&profile.executable)?;
    let workspace = canonical_workspace(&profile.workspace)?;
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_environment(&mut command);
    let bytes = run_bounded_probe(command, profile.preflight_timeout).await?;
    normalize_version(&bytes)
}

async fn probe_subscription_auth(profile: &CodexLaunchProfile) -> Result<(), CodexError> {
    let executable = canonical_executable(&profile.executable)?;
    let workspace = canonical_workspace(&profile.workspace)?;
    let mut command = Command::new(executable);
    command
        .args(["login", "status"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    sanitize_environment(&mut command);
    let bytes = run_bounded_probe(command, AUTH_PROBE_TIMEOUT).await?;
    validate_subscription_auth(&bytes)
}

async fn run_bounded_probe(mut command: Command, timeout: Duration) -> Result<Vec<u8>, CodexError> {
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
    let bytes = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(bytes)) if bytes.len() <= MAX_AUTH_OUTPUT_BYTES => bytes,
        _ => {
            if reap_probe(&mut child, process_group).await {
                return Err(authentication_required());
            }
            return Err(reap_failed());
        }
    };
    let status = match tokio::time::timeout(timeout, child.wait()).await {
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
    Ok(bytes)
}

fn normalize_version(bytes: &[u8]) -> Result<String, CodexError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| incompatible())?
        .trim();
    if output.lines().count() != 1 {
        return Err(incompatible());
    }
    let version = output
        .strip_prefix("codex-cli ")
        .or_else(|| output.strip_prefix("codex "))
        .unwrap_or(output)
        .trim();
    let mut components = version.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let minor = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    let patch = components
        .next()
        .and_then(|value| value.parse::<u64>().ok());
    if components.next().is_some()
        || !matches!((major, minor, patch), (Some(_), Some(_), Some(_)))
        || version.is_empty()
        || version.len() > MAX_VERSION_BYTES
        || version.chars().any(char::is_control)
    {
        return Err(incompatible());
    }
    if (
        major.unwrap_or_default(),
        minor.unwrap_or_default(),
        patch.unwrap_or_default(),
    ) < (0, 145, 0)
    {
        return Err(incompatible());
    }
    Ok(version.to_owned())
}

fn validate_subscription_auth(bytes: &[u8]) -> Result<(), CodexError> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| authentication_required())?
        .trim();
    if output.lines().count() > 8 || output.len() > MAX_AUTH_OUTPUT_BYTES {
        return Err(authentication_required());
    }
    let normalized = output.to_ascii_lowercase();
    if normalized.contains("logged in using chatgpt")
        && !normalized.contains("api key")
        && !normalized.contains("api-key")
    {
        Ok(())
    } else {
        Err(authentication_required())
    }
}

pub(crate) fn sanitize_environment(command: &mut Command) {
    // Never strip CODEX_HOME: the official CLI owns credential files there.
    // Workbench never opens those files; it only removes API/billing selectors.
    for name in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_API_BASE",
        "OPENAI_ORG_ID",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT",
        "CODEX_OSS_BASE_URL",
        "OLLAMA_BASE_URL",
        "OPENAI_API_KEY_PATH",
    ] {
        command.env_remove(name);
    }
}

fn validate_launch(
    profile: &CodexLaunchProfile,
    model: Option<&str>,
    prompt: Option<&str>,
) -> Result<(), CodexError> {
    if profile.preflight_timeout.is_zero()
        || profile.shutdown_grace.is_zero()
        || model.is_some_and(|model| {
            model.is_empty()
                || model.len() > 4_096
                || model.starts_with('-')
                || model.chars().any(char::is_control)
        })
        || prompt.is_some_and(|prompt| {
            prompt.is_empty()
                || prompt.len() > MAX_PROMPT_BYTES
                || prompt.chars().any(|ch| ch == '\0')
        })
    {
        Err(invalid_configuration())
    } else {
        Ok(())
    }
}

fn canonical_executable(path: &Path) -> Result<PathBuf, CodexError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid_configuration())?;
    if !canonical.is_absolute() || !canonical.is_file() {
        return Err(invalid_configuration());
    }
    Ok(canonical)
}

fn canonical_workspace(path: &Path) -> Result<PathBuf, CodexError> {
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

fn invalid_configuration() -> CodexError {
    CodexError::new(
        CodexErrorKind::InvalidConfiguration,
        "Codex launch configuration is invalid",
    )
}

fn spawn_failed() -> CodexError {
    CodexError::new(
        CodexErrorKind::SpawnFailed,
        "Codex process could not be started",
    )
}

fn authentication_required() -> CodexError {
    CodexError::new(
        CodexErrorKind::AuthenticationRequired,
        "Codex subscription authentication is unavailable",
    )
}

fn incompatible() -> CodexError {
    CodexError::new(
        CodexErrorKind::IncompatibleProtocol,
        "Codex protocol is incompatible",
    )
}

fn reap_failed() -> CodexError {
    CodexError::new(
        CodexErrorKind::ReapFailed,
        "Codex process could not be reaped",
    )
}

#[cfg(test)]
mod tests {
    use super::{normalize_version, validate_subscription_auth};

    #[test]
    fn normalizes_codex_cli_version() {
        assert_eq!(
            normalize_version(b"codex-cli 0.145.0\n").expect("version"),
            "0.145.0"
        );
        assert_eq!(normalize_version(b"0.145.1\n").expect("version"), "0.145.1");
        assert!(normalize_version(b"codex-cli 0.144.9\n").is_err());
    }

    #[test]
    fn accepts_only_chatgpt_login_status() {
        assert!(validate_subscription_auth(b"Logged in using ChatGPT\n").is_ok());
        assert!(validate_subscription_auth(b"Logged in using an API key\n").is_err());
        assert!(validate_subscription_auth(b"Not logged in\n").is_err());
        assert!(validate_subscription_auth(b"Logged in using unknown\n").is_err());
    }
}
