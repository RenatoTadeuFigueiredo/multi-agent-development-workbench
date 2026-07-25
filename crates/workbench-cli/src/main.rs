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
    providers::probe_configured_adapter_inputs,
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
        Err(LocalCommand::AgentStdio) => run_agent_stdio(&repository_root).await,
        Err(LocalCommand::ConfigValidate) => run_config(&cli, &repository_root, false).await,
        Err(LocalCommand::ConfigLock) => run_config(&cli, &repository_root, true).await,
        Ok(command) => {
            if cli.configuration.is_some() {
                return Err(CommandFailure::invalid_input(
                    "--configuration applies only to daemon and config commands",
                ));
            }
            run_remote(&cli, &repository_root, command).await
        }
    }
}

async fn run_agent_stdio(repository_root: &Path) -> Result<(), CommandFailure> {
    use std::io::{BufRead, Write};
    use std::sync::Arc;
    use workbench_acp_server::{AcpAgentServer, DaemonSocketBackend};

    let paths = RuntimePaths::discover(repository_root)
        .map_err(|error| CommandFailure::runtime_path(&error))?;
    let backend = DaemonSocketBackend::connect(&paths.endpoint)
        .await
        .map_err(|error| {
            CommandFailure::new(
                ExitCode::Internal,
                error.message().to_owned(),
                json!({
                    "code": "daemon_unavailable",
                    "message": error.message().to_owned(),
                }),
            )
        })?;
    let server = AcpAgentServer::new(Arc::new(backend));
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| {
            CommandFailure::new(
                ExitCode::Internal,
                format!("failed to read ACP stdin: {error}"),
                json!({
                    "code": "io_failure",
                    "message": format!("ACP stdin read failed: {error}"),
                }),
            )
        })?;
        let frames = server.handle_line(line.as_bytes()).await.map_err(|error| {
            CommandFailure::new(
                ExitCode::InvalidInput,
                error.message().to_owned(),
                json!({
                    "code": "invalid_request",
                    "message": error.message().to_owned(),
                }),
            )
        })?;
        for frame in frames {
            stdout
                .write_all(&frame)
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush())
                .map_err(|error| {
                    CommandFailure::new(
                        ExitCode::Internal,
                        format!("failed to write ACP stdout: {error}"),
                        json!({
                            "code": "io_failure",
                            "message": format!("ACP stdout write failed: {error}"),
                        }),
                    )
                })?;
        }
    }
    server.shutdown();
    Ok(())
}

async fn run_daemon(cli: &Cli, repository_root: &Path) -> Result<(), CommandFailure> {
    init_tracing();
    let paths = RuntimePaths::discover(repository_root)
        .map_err(|error| CommandFailure::runtime_path(&error))?;
    let startup = DaemonRuntime::start_with_configuration(
        &paths,
        repository_root,
        cli.configuration.as_deref(),
    );
    tokio::pin!(startup);
    let (runtime, interrupted) = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|error| CommandFailure::io(&error))?;
            (
                startup
                    .await
                    .map_err(|error| CommandFailure::runtime(&error))?,
                true,
            )
        }
        result = &mut startup => (
            result.map_err(|error| CommandFailure::runtime(&error))?,
            false,
        ),
    };
    if interrupted {
        runtime.1.shutdown();
        runtime
            .0
            .run()
            .await
            .map_err(|error| CommandFailure::runtime(&error))?;
    } else {
        runtime
            .0
            .run_until_signal()
            .await
            .map_err(|error| CommandFailure::runtime(&error))?;
    }
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

async fn run_config(
    cli: &Cli,
    repository_root: &Path,
    write_lock: bool,
) -> Result<(), CommandFailure> {
    let probes =
        StartupConfiguration::adapter_probes(repository_root, cli.configuration.as_deref())
            .map_err(|error| CommandFailure::configuration(&error))?;
    let inspected = if write_lock {
        let inputs = probe_configured_adapter_inputs(&probes, repository_root)
            .await
            .map_err(|error| CommandFailure::provider(&error))?;
        StartupConfiguration::inspect_with_adapter_inputs(
            repository_root,
            cli.configuration.as_deref(),
            &inputs,
        )
        .map_err(|error| CommandFailure::configuration(&error))?
    } else if probes.is_empty() {
        StartupConfiguration::inspect(repository_root, cli.configuration.as_deref())
            .map_err(|error| CommandFailure::configuration(&error))?
    } else {
        StartupConfiguration::load_with_configuration(repository_root, cli.configuration.as_deref())
            .map_err(|error| CommandFailure::configuration(&error))?
    };
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

async fn run_remote(
    cli: &Cli,
    repository_root: &Path,
    command: ClientCommand,
) -> Result<(), CommandFailure> {
    let paths = RuntimePaths::discover(repository_root)
        .map_err(|error| CommandFailure::runtime_path(&error))?;
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

    fn configuration(_error: &ConfigError) -> Self {
        let message = "configuration validation failed";
        Self::new(
            ExitCode::InvalidInput,
            message,
            json!({"code": "invalid_configuration", "message": message}),
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
            RuntimeError::Provider(_) => ExitCode::UnavailableCapability,
            RuntimeError::Telemetry(_) | RuntimeError::StartupTask => ExitCode::Internal,
        };
        Self::new(
            exit,
            error.to_string(),
            json!({"code": "daemon_startup_failed", "message": error.to_string()}),
        )
    }

    fn provider(error: &workbench_daemon::providers::ProviderRuntimeError) -> Self {
        Self::new(
            ExitCode::InvalidInput,
            error.to_string(),
            json!({"code": "provider_preflight_failed", "message": error.to_string()}),
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
