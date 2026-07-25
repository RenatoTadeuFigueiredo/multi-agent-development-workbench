use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "workbench", version, about = "Local multi-agent control plane")]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub request_id: Option<Uuid>,
    #[arg(long, global = true, value_name = "PATH")]
    pub configuration: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Daemon,
    /// ACP agent surfaces for editor and terminal clients.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Prompt(PromptArgs),
    Status(StatusArgs),
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Speak ACP v1 agent JSON-RPC NDJSON on stdio (bridge to the daemon).
    Stdio,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Validate,
    Lock,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    Create,
    List(ListArgs),
    Attach(AttachArgs),
    Pause(SessionIdArgs),
    Resume(SessionIdArgs),
    Cancel(SessionIdArgs),
    Redirect(RedirectArgs),
    Approve(ApproveArgs),
    Reconcile(ReconcileArgs),
    Export(ExportArgs),
    Delete(DeleteArgs),
}

#[derive(Debug, Args)]
pub struct SessionIdArgs {
    pub session_id: Uuid,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    pub session_id: Uuid,
    #[arg(long, default_value_t = 0)]
    pub after: u64,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long, default_value_t = 50, value_parser = parse_session_list_limit)]
    pub limit: u16,
    #[arg(long)]
    pub before_session_id: Option<Uuid>,
}

fn parse_session_list_limit(value: &str) -> Result<u16, String> {
    let limit = value
        .parse::<u16>()
        .map_err(|_| "limit must be an integer between 1 and 100".to_owned())?;
    if !(1..=100).contains(&limit) {
        return Err("limit must be between 1 and 100".to_owned());
    }
    Ok(limit)
}

#[derive(Debug, Args)]
pub struct PromptArgs {
    pub session_id: Uuid,
    #[arg(long)]
    pub role: Option<String>,
    #[arg(value_name = "TEXT")]
    pub text: String,
}

#[derive(Debug, Args)]
pub struct RedirectArgs {
    pub session_id: Uuid,
    pub instruction: String,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    pub session_id: Uuid,
    pub approval_id: Uuid,
    #[arg(long, value_enum)]
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ApprovalDecision {
    Grant,
    Deny,
}

#[derive(Debug, Args)]
pub struct ReconcileArgs {
    pub session_id: Uuid,
    pub attempt_id: Uuid,
    #[arg(value_enum)]
    pub resolution: Reconciliation,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Reconciliation {
    Retry,
    AcceptResult,
    Abandon,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    pub session_id: Uuid,
    #[arg(long = "recipient", required = true)]
    pub recipients: Vec<String>,
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    pub session_id: Uuid,
    #[arg(long)]
    pub confirm: Uuid,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    pub session_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_prompt_from_stdin_marker() {
        let cli = Cli::try_parse_from([
            "workbench",
            "prompt",
            "018f47ef-9052-7b86-b31d-3f8962457776",
            "-",
        ])
        .expect("valid prompt command");

        let Command::Prompt(prompt) = cli.command else {
            panic!("prompt command expected");
        };
        assert_eq!(prompt.text, "-");
    }

    #[test]
    fn parses_exact_delete_confirmation_value() {
        let cli = Cli::try_parse_from([
            "workbench",
            "session",
            "delete",
            "018f47ef-9052-7b86-b31d-3f8962457776",
            "--confirm",
            "018f47ef-9052-7b86-b31d-3f8962457776",
        ])
        .expect("valid delete command");

        let Command::Session {
            command: SessionCommand::Delete(delete),
        } = cli.command
        else {
            panic!("delete command expected");
        };
        assert_eq!(delete.session_id, delete.confirm);
    }

    #[test]
    fn parses_bounded_session_list_cursor() {
        let cli = Cli::try_parse_from([
            "workbench",
            "session",
            "list",
            "--limit",
            "20",
            "--before-session-id",
            "018f47ef-9052-7b86-b31d-3f8962457776",
        ])
        .expect("valid session list command");

        let Command::Session {
            command: SessionCommand::List(list),
        } = cli.command
        else {
            panic!("session list command expected");
        };
        assert_eq!(list.limit, 20);
        assert_eq!(
            list.before_session_id,
            Some(Uuid::parse_str("018f47ef-9052-7b86-b31d-3f8962457776").expect("static UUID"))
        );
    }

    #[test]
    fn rejects_session_list_limit_outside_the_protocol_bound() {
        assert!(Cli::try_parse_from(["workbench", "session", "list", "--limit", "0"]).is_err());
        assert!(Cli::try_parse_from(["workbench", "session", "list", "--limit", "101"]).is_err());
    }
}
