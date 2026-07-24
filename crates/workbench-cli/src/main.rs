use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use clap::Parser;
use serde_json::{Value, json};
use uuid::Uuid;
use workbench_cli::{
    args::{Cli, Command},
    client::{ClientError, ProtocolClient},
    commands::{CommandBuildError, LocalCommand, owns_active_prompt, resolve},
    output::{self, ExitCode},
};
use workbench_config::ConfigError;
use workbench_daemon::{
    DaemonRuntime, StartupConfiguration,
    runtime::{RuntimeError, init_tracing},
    runtime_paths::{RuntimePathError, RuntimePaths},
};
use workbench_protocol::{
    ClientCommand, Command as ProtocolCommand, PROTOCOL_V1, ProtocolError, command::EmptyParams,
};

#[tokio::main]
async fn main() {
    let mut cli = Cli::parse();
    cli.request_id.get_or_insert_with(Uuid::now_v7);
    let json_output = cli.json;
    let request_id = cli.request_id.expect("request ID is populated");

    let exit = match run(cli).await {
        Ok(()) => ExitCode::Success,
        Err(failure) => {
            if json_output {
                let envelope = output::failure(request_id, &failure.error);
                if let Err(error) = write_stdout(&envelope, false) {
                    eprintln!("failed to write JSON failure: {}", error.message);
                    std::process::exit(ExitCode::Internal as i32);
                }
            } else {
                eprintln!("{}", failure.message);
            }
            failure.exit
        }
    };
    std::process::exit(exit as i32);
}

async fn run(cli: Cli) -> Result<(), CommandFailure> {
    let repository_root = repository_root()?;
    let stdin_prompt = read_prompt_stdin_if_requested(&cli)?;
    let command = resolve(&cli, stdin_prompt).map_err(CommandFailure::invalid_input)?;
    match command {
        Err(LocalCommand::Daemon) => run_daemon(&cli, &repository_root).await,
        Err(LocalCommand::ConfigValidate) => run_config(&cli, &repository_root, false),
        Err(LocalCommand::ConfigLock) => run_config(&cli, &repository_root, true),
        Ok(command) => {
            if cli.configuration.is_some() {
                return Err(CommandFailure::invalid_input(
                    "--configuration applies only to daemon and config commands",
                ));
            }
            run_remote(&cli, command).await
        }
    }
}

async fn run_daemon(cli: &Cli, repository_root: &Path) -> Result<(), CommandFailure> {
    init_tracing();
    let paths = RuntimePaths::discover().map_err(|error| CommandFailure::runtime_path(&error))?;
    let runtime = DaemonRuntime::start_with_configuration(
        &paths,
        repository_root,
        cli.configuration.as_deref(),
    )
    .map_err(|error| CommandFailure::runtime(&error))?;
    runtime
        .0
        .run_until_signal()
        .await
        .map_err(|error| CommandFailure::runtime(&error))?;
    if cli.json {
        write_stdout(
            &output::success(
                cli.request_id.expect("request ID"),
                &json!({"state": "stopped"}),
            ),
            false,
        )?;
    }
    Ok(())
}

fn run_config(cli: &Cli, repository_root: &Path, write_lock: bool) -> Result<(), CommandFailure> {
    let inspected = StartupConfiguration::inspect(repository_root, cli.configuration.as_deref())
        .map_err(|error| CommandFailure::configuration(&error))?;
    if write_lock {
        inspected
            .write_base_lock(repository_root)
            .map_err(|error| CommandFailure::configuration(&error))?;
    }
    let result = json!({
        "configuration_hash": inspected.snapshot.content_hash,
        "lock_hash": inspected.base_lock.hash().map_err(|error| CommandFailure::configuration(&error))?,
        "sources": inspected.sources,
        "lock_written": write_lock,
    });
    print_result(cli, &result)
}

async fn run_remote(cli: &Cli, command: ClientCommand) -> Result<(), CommandFailure> {
    let paths = RuntimePaths::discover().map_err(|error| CommandFailure::runtime_path(&error))?;
    let request_id = command.request_id;
    let is_attach = matches!(command.command, ProtocolCommand::SessionAttach(_));
    let prompt_session = if owns_active_prompt(&command) {
        Some(command.session_id.expect("prompt has a session ID"))
    } else {
        None
    };
    let mut client = ProtocolClient::connect(&paths.endpoint)
        .await
        .map_err(CommandFailure::client)?;

    let result = if let Some(session_id) = prompt_session {
        let mut prompt = Box::pin(client.call_validated(command));
        tokio::select! {
            biased;
            result = &mut prompt => result.map_err(CommandFailure::client)?,
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| CommandFailure::io(&error))?;
                match request_owned_prompt_cancellation(&paths.endpoint, session_id).await {
                    Ok((request_id, result)) => {
                        print_result_with_id(cli, request_id, &result)?;
                        return Ok(());
                    }
                    Err(ClientError::Protocol(error))
                        if error.code == workbench_protocol::ErrorCode::InvalidTransition =>
                    {
                        prompt.await.map_err(CommandFailure::client)?
                    }
                    Err(error) => return Err(CommandFailure::client(error)),
                }
            }
        }
    } else {
        client
            .call_validated(command)
            .await
            .map_err(CommandFailure::client)?
    };
    print_result_with_id(cli, request_id, &result)?;

    if is_attach {
        follow_events(cli, &mut client).await?;
    }
    Ok(())
}

async fn request_owned_prompt_cancellation(
    endpoint: &Path,
    session_id: Uuid,
) -> Result<(Uuid, Value), ClientError> {
    let mut client = ProtocolClient::connect(endpoint).await?;
    let request_id = Uuid::now_v7();
    let result = client
        .call_validated(ClientCommand {
            protocol: PROTOCOL_V1.to_owned(),
            request_id,
            session_id: Some(session_id),
            command: ProtocolCommand::SessionCancel(EmptyParams::default()),
        })
        .await?;
    Ok((request_id, result))
}

async fn follow_events(cli: &Cli, client: &mut ProtocolClient) -> Result<(), CommandFailure> {
    loop {
        tokio::select! {
            event = client.next_event() => {
                let event = event.map_err(CommandFailure::client)?;
                let value = if cli.json {
                    output::event(&event)
                } else {
                    serde_json::to_value(&event).map_err(|error| CommandFailure::json(&error))?
                };
                write_stdout(&value, !cli.json)?;
            }
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| CommandFailure::io(&error))?;
                return Ok(());
            }
        }
    }
}

fn print_result(cli: &Cli, result: &Value) -> Result<(), CommandFailure> {
    print_result_with_id(cli, cli.request_id.expect("request ID"), result)
}

fn print_result_with_id(cli: &Cli, request_id: Uuid, result: &Value) -> Result<(), CommandFailure> {
    let value = if cli.json {
        output::success(request_id, result)
    } else {
        result.clone()
    };
    write_stdout(&value, !cli.json)
}

fn write_stdout(value: &Value, pretty: bool) -> Result<(), CommandFailure> {
    let encoded = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|error| CommandFailure::json(&error))?;
    let stdout = io::stdout();
    let mut locked = stdout.lock();
    writeln!(locked, "{encoded}").map_err(|error| CommandFailure::io(&error))?;
    locked.flush().map_err(|error| CommandFailure::io(&error))
}

fn read_prompt_stdin_if_requested(cli: &Cli) -> Result<Option<String>, CommandFailure> {
    if !matches!(&cli.command, Command::Prompt(prompt) if prompt.text == "-") {
        return Ok(None);
    }
    let mut prompt = String::new();
    io::stdin()
        .read_to_string(&mut prompt)
        .map_err(|error| CommandFailure::io(&error))?;
    Ok(Some(prompt))
}

fn repository_root() -> Result<PathBuf, CommandFailure> {
    std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|error| CommandFailure::io(&error))
}

struct CommandFailure {
    exit: ExitCode,
    message: String,
    error: Value,
}

impl CommandFailure {
    fn new(exit: ExitCode, message: impl Into<String>, error: Value) -> Self {
        Self {
            exit,
            message: message.into(),
            error,
        }
    }

    fn invalid_input(error: impl std::fmt::Display) -> Self {
        Self::new(
            ExitCode::InvalidInput,
            error.to_string(),
            json!({"code": "invalid_input", "message": error.to_string()}),
        )
    }

    fn configuration(error: &ConfigError) -> Self {
        Self::new(
            ExitCode::InvalidInput,
            error.to_string(),
            json!({"code": "invalid_configuration", "message": error.to_string()}),
        )
    }

    fn runtime_path(error: &RuntimePathError) -> Self {
        Self::new(
            ExitCode::ProtocolFailure,
            error.to_string(),
            json!({"code": "runtime_path_unavailable", "message": error.to_string()}),
        )
    }

    fn runtime(error: &RuntimeError) -> Self {
        let exit = match error {
            RuntimeError::Configuration(_) => ExitCode::InvalidInput,
            RuntimeError::Storage(_) => ExitCode::StorageFailure,
            RuntimeError::RuntimePath(_) | RuntimeError::Io(_) => ExitCode::ProtocolFailure,
            RuntimeError::Telemetry(_) => ExitCode::Internal,
        };
        Self::new(
            exit,
            error.to_string(),
            json!({"code": "daemon_startup_failed", "message": error.to_string()}),
        )
    }

    fn client(error: ClientError) -> Self {
        if let ClientError::Protocol(protocol) = error {
            return Self::protocol(protocol);
        }
        Self::new(
            ExitCode::ProtocolFailure,
            error.to_string(),
            json!({"code": "protocol_failure", "message": error.to_string()}),
        )
    }

    fn protocol(error: ProtocolError) -> Self {
        let exit = ExitCode::from_protocol(error.code);
        let value = serde_json::to_value(&error).unwrap_or_else(
            |_| json!({"code": "internal", "message": "failed to serialize protocol error"}),
        );
        Self::new(exit, error.message, value)
    }

    fn io(error: &io::Error) -> Self {
        Self::new(
            ExitCode::Internal,
            "local I/O failed",
            json!({"code": "io_failure", "message": error.to_string()}),
        )
    }

    fn json(error: &serde_json::Error) -> Self {
        Self::new(
            ExitCode::Internal,
            "JSON serialization failed",
            json!({"code": "json_failure", "message": error.to_string()}),
        )
    }
}

impl From<CommandBuildError> for CommandFailure {
    fn from(error: CommandBuildError) -> Self {
        Self::invalid_input(error)
    }
}
