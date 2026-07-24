//! Supervised local stdio MCP children.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
    task::JoinHandle,
};
use workbench_config::model::McpServer;

use crate::{
    error::{McpError, outcome_unknown, reap_failed, transport_failed, unavailable},
    pin::canonicalize_mcp_executable,
};

const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_millis(100);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Workspace-scoped supervised stdio MCP child.
pub struct StdioChild {
    server_id: String,
    workspace_key: String,
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    stderr: JoinHandle<()>,
    shutdown_grace: Duration,
    next_id: u64,
}

/// Report from terminating a supervised child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildShutdownReport {
    pub reaped: bool,
    pub forced: bool,
}

impl StdioChild {
    /// Spawns a direct-argv MCP child with private working directory.
    ///
    /// # Errors
    ///
    /// Returns when the executable is unsafe or spawn fails.
    pub async fn spawn(
        server_id: impl Into<String>,
        workspace_key: impl Into<String>,
        server: &McpServer,
        runtime_dir: &Path,
    ) -> Result<Self, McpError> {
        let server_id = server_id.into();
        let workspace_key = workspace_key.into();
        let executable = server.executable.as_deref().ok_or_else(unavailable)?;
        let executable =
            canonicalize_mcp_executable(Path::new(executable)).map_err(|_| unavailable())?;
        let work_dir = private_runtime_dir(runtime_dir, &workspace_key, &server_id);
        tokio::fs::create_dir_all(&work_dir)
            .await
            .map_err(|_| unavailable())?;
        let mut command = Command::new(&executable);
        command
            .args(&server.args)
            .current_dir(&work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        sanitize_environment(&mut command);
        // Secret handles are never expanded into the child environment by the
        // gateway; only opaque presence is asserted offline.
        for key in server.env.keys() {
            command.env_remove(key);
        }
        let mut child = command.spawn().map_err(|_| transport_failed())?;
        let stdin = child.stdin.take().ok_or_else(transport_failed)?;
        let stdout = child.stdout.take().ok_or_else(transport_failed)?;
        let stderr = child.stderr.take().ok_or_else(transport_failed)?;
        Ok(Self {
            server_id,
            workspace_key,
            child,
            stdin,
            reader: BufReader::new(stdout),
            stderr: spawn_stderr_drain(stderr),
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            next_id: 1,
        })
    }

    #[must_use]
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    #[must_use]
    pub fn workspace_key(&self) -> &str {
        &self.workspace_key
    }

    /// Invokes a named tool operation through a narrow JSON-RPC envelope.
    ///
    /// # Errors
    ///
    /// Returns transport or size failures without leaking child output.
    pub async fn invoke(
        &mut self,
        operation: &str,
        arguments: &Value,
        max_frame_bytes: usize,
    ) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": operation,
                "arguments": arguments,
            }
        });
        let encoded = serde_json::to_vec(&request).map_err(|_| transport_failed())?;
        if encoded.len() > max_frame_bytes {
            return Err(crate::error::response_too_large());
        }
        self.stdin
            .write_all(&encoded)
            .await
            .map_err(|_| transport_failed())?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|_| transport_failed())?;
        self.stdin.flush().await.map_err(|_| transport_failed())?;

        let mut line = String::new();
        let read = tokio::time::timeout(DEFAULT_CALL_TIMEOUT, self.reader.read_line(&mut line))
            .await
            .map_err(|_| {
                McpError::new(
                    crate::error::McpErrorKind::Timeout,
                    "MCP stdio call timed out",
                )
            })?
            .map_err(|_| transport_failed())?;
        if read == 0 {
            return Err(transport_failed());
        }
        if line.len() > max_frame_bytes {
            return Err(crate::error::response_too_large());
        }
        let value: Value = serde_json::from_str(line.trim_end()).map_err(|_| transport_failed())?;
        if value.get("error").is_some() {
            return Err(transport_failed());
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Terminates and reaps the child process group.
    pub async fn shutdown(mut self) -> ChildShutdownReport {
        let _ = self.stdin.shutdown().await;
        let process_group = self
            .child
            .id()
            .and_then(|raw| Pid::from_raw(raw.cast_signed()));
        let mut forced = false;
        if let Some(pid) = process_group {
            let _ = kill_process_group(pid, Signal::TERM);
        }
        let child_reaped = if wait_for_exit(&mut self.child, self.shutdown_grace).await {
            true
        } else {
            if let Some(pid) = process_group {
                let _ = kill_process_group(pid, Signal::KILL);
            }
            forced = true;
            let _ = self.child.start_kill();
            wait_for_exit(&mut self.child, self.shutdown_grace).await
        };
        let group_reaped = if let Some(pid) = process_group {
            if test_kill_process_group(pid).is_err() {
                true
            } else {
                let _ = kill_process_group(pid, Signal::KILL);
                tokio::time::sleep(self.shutdown_grace).await;
                test_kill_process_group(pid).is_err()
            }
        } else {
            true
        };
        self.stderr.abort();
        ChildShutdownReport {
            reaped: child_reaped && group_reaped,
            forced,
        }
    }
}

async fn wait_for_exit(child: &mut Child, grace: Duration) -> bool {
    matches!(tokio::time::timeout(grace, child.wait()).await, Ok(Ok(_)))
}

fn sanitize_environment(command: &mut Command) {
    command.env_remove("HTTP_PROXY");
    command.env_remove("HTTPS_PROXY");
    command.env_remove("ALL_PROXY");
    command.env_remove("http_proxy");
    command.env_remove("https_proxy");
    command.env_remove("all_proxy");
}

fn spawn_stderr_drain(stderr: tokio::process::ChildStderr) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buffer = vec![0_u8; 4_096];
        let mut total = 0_usize;
        loop {
            match tokio::io::AsyncReadExt::read(&mut reader, &mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    total = total.saturating_add(read);
                    if total >= MAX_STDERR_BYTES {
                        break;
                    }
                }
            }
        }
    })
}

/// Pool of workspace-isolated stdio children.
#[derive(Default)]
pub struct StdioPool {
    children: Mutex<HashMap<(String, String), StdioChild>>,
}

impl StdioPool {
    #[must_use]
    pub fn new() -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
        }
    }

    /// Returns an existing child or spawns a new one for the workspace.
    ///
    /// # Errors
    ///
    /// Returns when the executable is unsafe or spawn fails.
    pub async fn get_or_spawn(
        &self,
        server_id: &str,
        workspace_key: &str,
        server: &McpServer,
        runtime_dir: &Path,
    ) -> Result<(), McpError> {
        let key = (workspace_key.to_owned(), server_id.to_owned());
        let mut guard = self.children.lock().await;
        if guard.contains_key(&key) {
            return Ok(());
        }
        let child = StdioChild::spawn(server_id, workspace_key, server, runtime_dir).await?;
        guard.insert(key, child);
        Ok(())
    }

    /// Invokes a tool on the workspace-scoped child.
    ///
    /// # Errors
    ///
    /// Returns when the child is missing, the call times out, or transport fails.
    pub async fn invoke(
        &self,
        server_id: &str,
        workspace_key: &str,
        operation: &str,
        arguments: &Value,
    ) -> Result<Value, McpError> {
        let key = (workspace_key.to_owned(), server_id.to_owned());
        let mut guard = self.children.lock().await;
        let child = guard.get_mut(&key).ok_or_else(unavailable)?;
        child
            .invoke(operation, arguments, DEFAULT_MAX_FRAME_BYTES)
            .await
    }

    /// Shuts down every child, optionally filtered by workspace.
    ///
    /// # Errors
    ///
    /// Returns when any selected child cannot be reaped.
    pub async fn shutdown(&self, workspace_key: Option<&str>) -> Result<(), McpError> {
        let mut guard = self.children.lock().await;
        let keys = guard
            .keys()
            .filter(|(workspace, _)| workspace_key.is_none_or(|filter| workspace == filter))
            .cloned()
            .collect::<Vec<_>>();
        let mut failed = false;
        for key in keys {
            if let Some(child) = guard.remove(&key) {
                let report = child.shutdown().await;
                if !report.reaped {
                    failed = true;
                }
            }
        }
        if failed { Err(reap_failed()) } else { Ok(()) }
    }

    #[must_use]
    pub async fn active_count(&self) -> usize {
        self.children.lock().await.len()
    }

    #[must_use]
    pub async fn active_for_workspace(&self, workspace_key: &str) -> usize {
        self.children
            .lock()
            .await
            .keys()
            .filter(|(workspace, _)| workspace == workspace_key)
            .count()
    }
}

/// Shared pool handle.
pub type SharedStdioPool = Arc<StdioPool>;

/// Maps cancel-after-start into `outcome_unknown`.
#[must_use]
pub const fn cancel_after_start() -> McpError {
    outcome_unknown()
}

/// Runtime working directory helper for tests.
#[must_use]
pub fn private_runtime_dir(root: impl Into<PathBuf>, workspace: &str, server: &str) -> PathBuf {
    root.into().join("mcp").join(workspace).join(server)
}
