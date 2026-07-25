use thiserror::Error;
use uuid::Uuid;
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, PROTOCOL_V1,
    command::{
        ApprovalDecision as ProtocolApprovalDecision, ApprovalParams, AttachSessionParams,
        CreateSessionParams, DeleteParams, EmptyParams, ExportParams, ListSessionsParams,
        PromptParams, ReconciliationParams, ReconciliationResolution, RedirectParams,
    },
};

use crate::args::{
    AgentCommand, ApprovalDecision, Cli, Command, ConfigCommand, Reconciliation, SessionCommand,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LocalCommand {
    Daemon,
    AgentStdio,
    ConfigValidate,
    ConfigLock,
}

#[must_use]
pub fn owns_active_prompt(command: &ClientCommand) -> bool {
    matches!(command.command, ProtocolCommand::SessionPrompt(_))
}

/// Converts parsed CLI input into a local action or one protocol request.
///
/// `stdin_prompt` is required only when the prompt argument is `-`.
///
/// # Errors
///
/// Returns an error for missing standard-input content, mismatched deletion
/// confirmation, or a non-UTF-8 export path.
pub fn resolve(
    cli: &Cli,
    stdin_prompt: Option<String>,
) -> Result<Result<ClientCommand, LocalCommand>, CommandBuildError> {
    let request_id = cli.request_id.unwrap_or_else(Uuid::now_v7);
    let (session_id, command) = match &cli.command {
        Command::Daemon => return Ok(Err(LocalCommand::Daemon)),
        Command::Agent { command } => {
            return Ok(Err(match command {
                AgentCommand::Stdio => LocalCommand::AgentStdio,
            }));
        }
        Command::Config { command } => {
            return Ok(Err(match command {
                ConfigCommand::Validate => LocalCommand::ConfigValidate,
                ConfigCommand::Lock => LocalCommand::ConfigLock,
            }));
        }
        Command::Session { command } => session_command(command)?,
        Command::Prompt(prompt) => {
            let text = if prompt.text == "-" {
                stdin_prompt.ok_or(CommandBuildError::MissingStdin)?
            } else {
                prompt.text.clone()
            };
            (
                Some(prompt.session_id),
                ProtocolCommand::SessionPrompt(PromptParams {
                    text,
                    explicit_target: prompt.role.clone(),
                }),
            )
        }
        Command::Status(status) => status.session_id.map_or_else(
            || (None, ProtocolCommand::StatusGet(EmptyParams::default())),
            |session_id| {
                (
                    Some(session_id),
                    ProtocolCommand::SessionGet(EmptyParams::default()),
                )
            },
        ),
    };
    Ok(Ok(ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id,
        session_id,
        command,
    }))
}

fn session_command(
    command: &SessionCommand,
) -> Result<(Option<Uuid>, ProtocolCommand), CommandBuildError> {
    let mapped = match command {
        SessionCommand::Create => (
            None,
            ProtocolCommand::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
                workflow: None,
            }),
        ),
        SessionCommand::List(args) => (
            None,
            ProtocolCommand::SessionList(ListSessionsParams {
                limit: args.limit,
                before_session_id: args.before_session_id,
            }),
        ),
        SessionCommand::Attach(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionAttach(AttachSessionParams {
                after_sequence: args.after,
            }),
        ),
        SessionCommand::Pause(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionPause(EmptyParams::default()),
        ),
        SessionCommand::Resume(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionResume(EmptyParams::default()),
        ),
        SessionCommand::Cancel(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionCancel(EmptyParams::default()),
        ),
        SessionCommand::Redirect(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionRedirect(RedirectParams {
                instruction: args.instruction.clone(),
            }),
        ),
        SessionCommand::Approve(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionApprovalResolve(ApprovalParams {
                approval_id: args.approval_id,
                decision: match args.decision {
                    ApprovalDecision::Grant => ProtocolApprovalDecision::Grant,
                    ApprovalDecision::Deny => ProtocolApprovalDecision::Deny,
                },
            }),
        ),
        SessionCommand::Reconcile(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionReconcile(ReconciliationParams {
                attempt_id: args.attempt_id,
                resolution: match args.resolution {
                    Reconciliation::Retry => ReconciliationResolution::Retry,
                    Reconciliation::AcceptResult => ReconciliationResolution::AcceptResult,
                    Reconciliation::Abandon => ReconciliationResolution::Abandon,
                },
                evidence: None,
            }),
        ),
        SessionCommand::Export(args) => (
            Some(args.session_id),
            ProtocolCommand::SessionExport(ExportParams {
                output_path: args
                    .output
                    .to_str()
                    .ok_or(CommandBuildError::NonUtf8Path)?
                    .to_owned(),
                age_recipients: args.recipients.clone(),
            }),
        ),
        SessionCommand::Delete(args) => {
            if args.confirm != args.session_id {
                return Err(CommandBuildError::ConfirmationMismatch);
            }
            (
                Some(args.session_id),
                ProtocolCommand::SessionDelete(DeleteParams {
                    confirm_session_id: args.confirm,
                }),
            )
        }
    };
    Ok(mapped)
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CommandBuildError {
    #[error("prompt text '-' requires content on standard input")]
    MissingStdin,
    #[error("delete confirmation must exactly match the session identifier")]
    ConfirmationMismatch,
    #[error("export output path must be valid UTF-8")]
    NonUtf8Path,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn maps_status_to_a_read_only_daemon_command() {
        let cli = Cli::try_parse_from(["workbench", "status"]).expect("status command");
        let request = resolve(&cli, None)
            .expect("valid mapping")
            .expect("remote command");

        assert!(matches!(request.command, ProtocolCommand::StatusGet(_)));
        assert!(request.session_id.is_none());
    }

    #[test]
    fn reads_prompt_only_from_explicit_stdin_input() {
        let cli = Cli::try_parse_from([
            "workbench",
            "prompt",
            "018f47ef-9052-7b86-b31d-3f8962457776",
            "-",
        ])
        .expect("prompt command");

        assert_eq!(
            resolve(&cli, None).expect_err("stdin is required"),
            CommandBuildError::MissingStdin
        );
        let request = resolve(&cli, Some("review the change".to_owned()))
            .expect("valid mapping")
            .expect("remote command");
        let ProtocolCommand::SessionPrompt(prompt) = request.command else {
            panic!("prompt command expected");
        };
        assert_eq!(prompt.text, "review the change");
    }

    #[test]
    fn rejects_mismatched_delete_confirmation_locally() {
        let cli = Cli::try_parse_from([
            "workbench",
            "session",
            "delete",
            "018f47ef-9052-7b86-b31d-3f8962457776",
            "--confirm",
            "018f47ef-9052-7b86-b31d-3f8962457777",
        ])
        .expect("delete command");

        assert_eq!(
            resolve(&cli, None).expect_err("mismatch must fail"),
            CommandBuildError::ConfirmationMismatch
        );
    }

    #[test]
    fn only_prompt_commands_own_interrupt_cancellation() {
        let prompt = Cli::try_parse_from([
            "workbench",
            "prompt",
            "018f47ef-9052-7b86-b31d-3f8962457776",
            "review",
        ])
        .expect("prompt command");
        let status = Cli::try_parse_from(["workbench", "status"]).expect("status command");

        assert!(owns_active_prompt(
            &resolve(&prompt, None)
                .expect("prompt mapping")
                .expect("remote prompt")
        ));
        assert!(!owns_active_prompt(
            &resolve(&status, None)
                .expect("status mapping")
                .expect("remote status")
        ));
    }

    #[test]
    fn maps_session_list_without_a_session_envelope() {
        let cli = Cli::try_parse_from([
            "workbench",
            "session",
            "list",
            "--limit",
            "20",
            "--before-session-id",
            "018f47ef-9052-7b86-b31d-3f8962457776",
        ])
        .expect("session list command");

        let request = resolve(&cli, None)
            .expect("valid mapping")
            .expect("remote command");
        assert!(request.session_id.is_none());
        assert!(matches!(
            request.command,
            ProtocolCommand::SessionList(ListSessionsParams {
                limit: 20,
                before_session_id: Some(_),
            })
        ));
    }
}
