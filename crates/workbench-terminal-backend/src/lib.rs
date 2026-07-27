//! `WorkbenchBackend` — selectable external ACP backend for the Grok-derived
//! terminal integration path.
//!
//! Architecture ([`docs/architecture/grok-build-terminal-integration.md`](../../../docs/architecture/grok-build-terminal-integration.md)):
//!
//! ```text
//! AgentBackend
//! |-- GrokShellBackend        # upstream (out of tree)
//! `-- WorkbenchBackend        # this crate: launches workbench agent stdio
//! ```
//!
//! This crate intentionally does **not** fork pager rendering. It provides the
//! launch contract the Grok Build `workbench` branch plugs in beside
//! `GrokShellBackend`. Orchestration remains in the Workbench daemon.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Compatibility pin for the tested Grok Build fork integration commit.
///
/// Operators and the fork sync job record the matching upstream SHA here when
/// the downstream patch stack is rebased. Empty means "integration contract
/// only; fork pin not yet published from a dual-upstream rebase".
///
/// Current pin: recommended Mode C binary is **fcustom + Mode C**
/// (`feature/fcustom-mode-c` tip `f8468e5…`) — Fabricio custom line
/// (Claude CLI, providers) plus `WorkbenchBackend`. Previous pin
/// `85989c9…` was monorepo-sync Mode C only (`WorkbenchBackend` without
/// the fcustom stack). See
/// <https://github.com/RenatoTadeuFigueiredo/grok-build>.
pub const GROK_BUILD_FORK_COMPATIBILITY_PIN: &str = "f8468e52db1aeca4804341cc4ec88de57e496407";

/// Documented CLI subcommand the terminal launches.
pub const WORKBENCH_AGENT_STDIO_ARGS: &[&str] = &["agent", "stdio"];

/// Errors from constructing or validating a `WorkbenchBackend` launch.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkbenchBackendError {
    #[error("workbench executable path is empty")]
    EmptyExecutable,
    #[error("workbench executable path is not absolute")]
    RelativeExecutable,
    #[error("workbench executable path must not contain parent traversal")]
    ParentTraversal,
    #[error("workbench agent stdio launch is invalid: {0}")]
    InvalidLaunch(&'static str),
}

/// Launch configuration for the Workbench ACP agent stdio bridge.
///
/// The Grok-derived terminal selects this backend instead of
/// `GrokShellBackend` when Workbench orchestration is desired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchBackend {
    /// Absolute path to the `workbench` CLI binary.
    executable: PathBuf,
    /// Working directory for the child (workspace root).
    workspace: PathBuf,
}

impl WorkbenchBackend {
    /// Builds a backend that will launch `executable agent stdio`.
    ///
    /// # Errors
    ///
    /// Returns when the executable is empty, relative, or contains `..`.
    pub fn new(
        executable: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
    ) -> Result<Self, WorkbenchBackendError> {
        let executable = executable.into();
        let workspace = workspace.into();
        validate_absolute(&executable)?;
        validate_absolute(&workspace)?;
        Ok(Self {
            executable,
            workspace,
        })
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Argv for the ACP agent stdio child (excluding the executable).
    #[must_use]
    pub fn agent_stdio_args() -> Vec<OsString> {
        WORKBENCH_AGENT_STDIO_ARGS
            .iter()
            .map(OsString::from)
            .collect()
    }

    /// Builds a `Command` that launches `workbench agent stdio` in the workspace.
    ///
    /// The caller owns PTY attachment and process lifetime (Grok pager).
    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .args(WORKBENCH_AGENT_STDIO_ARGS)
            .current_dir(&self.workspace)
            .env("WORKBENCH_TERMINAL_BACKEND", "1");
        command
    }

    /// Validates the launch plan without spawning (offline unit path).
    ///
    /// # Errors
    ///
    /// Returns when the planned argv would not match the stdio contract.
    pub fn validate_launch_plan(&self) -> Result<(), WorkbenchBackendError> {
        let args = Self::agent_stdio_args();
        if args.len() != 2 {
            return Err(WorkbenchBackendError::InvalidLaunch(
                "expected exactly two args: agent stdio",
            ));
        }
        if args[0] != "agent" || args[1] != "stdio" {
            return Err(WorkbenchBackendError::InvalidLaunch(
                "args must be agent stdio",
            ));
        }
        // Keep path fields reachable for callers that probe configuration.
        let _ = (self.executable(), self.workspace());
        Ok(())
    }
}

fn validate_absolute(path: &Path) -> Result<(), WorkbenchBackendError> {
    if path.as_os_str().is_empty() {
        return Err(WorkbenchBackendError::EmptyExecutable);
    }
    if !path.is_absolute() {
        return Err(WorkbenchBackendError::RelativeExecutable);
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(WorkbenchBackendError::ParentTraversal);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_relative_and_empty_executable() {
        assert_eq!(
            WorkbenchBackend::new("", "/tmp/ws").expect_err("empty"),
            WorkbenchBackendError::EmptyExecutable
        );
        assert_eq!(
            WorkbenchBackend::new("workbench", "/tmp/ws").expect_err("relative"),
            WorkbenchBackendError::RelativeExecutable
        );
        assert_eq!(
            WorkbenchBackend::new("/opt/bin/../workbench", "/tmp/ws").expect_err("parent"),
            WorkbenchBackendError::ParentTraversal
        );
    }

    #[test]
    fn plans_agent_stdio_launch() {
        let backend = WorkbenchBackend::new(
            PathBuf::from("/usr/local/bin/workbench"),
            PathBuf::from("/workspace/repo"),
        )
        .expect("absolute paths");
        backend.validate_launch_plan().expect("plan");
        assert_eq!(
            WorkbenchBackend::agent_stdio_args(),
            vec![OsString::from("agent"), OsString::from("stdio")]
        );
        let program = backend.command();
        assert_eq!(program.get_program(), "/usr/local/bin/workbench");
        let args: Vec<_> = program.get_args().collect();
        assert_eq!(args, ["agent", "stdio"]);
        assert_eq!(
            program.get_current_dir().expect("cwd"),
            Path::new("/workspace/repo")
        );
    }

    #[test]
    fn compatibility_pin_is_documented_string() {
        // Published pin: Grok Build WorkbenchBackend integration commit.
        assert!(
            !GROK_BUILD_FORK_COMPATIBILITY_PIN.is_empty(),
            "GROK_BUILD_FORK_COMPATIBILITY_PIN must be a non-empty fork SHA"
        );
        assert_eq!(GROK_BUILD_FORK_COMPATIBILITY_PIN.len(), 40);
        assert!(
            GROK_BUILD_FORK_COMPATIBILITY_PIN
                .chars()
                .all(|c| c.is_ascii_hexdigit()),
            "pin must be a full lowercase hex commit SHA"
        );
        assert!(WORKBENCH_AGENT_STDIO_ARGS.contains(&"agent"));
        assert!(WORKBENCH_AGENT_STDIO_ARGS.contains(&"stdio"));
    }
}
