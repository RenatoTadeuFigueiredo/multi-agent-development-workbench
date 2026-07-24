use std::{collections::BTreeSet, time::Duration};

use serde_json::Value;
use uuid::Uuid;
use workbench_daemon::{Application, FakeBehavior, StartupConfiguration};
use workbench_protocol::{
    ClientCommand, Command, ErrorCode, PROTOCOL_V1,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, DeleteParams,
        EmptyParams, ExportParams, ListSessionsParams, PromptParams, ReconciliationParams,
        ReconciliationResolution, RedirectParams,
    },
    response::{
        ApprovalResult, AttachSessionResult, ControlResult, CreateSessionResult, ExportResult,
        ListSessionsResult, PromptResult, SessionResult, StatusResult,
    },
};

use crate::client::{LocalDaemonHarness, ProtocolTestClient, TestClientError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientContractReport {
    pub methods: BTreeSet<&'static str>,
    pub observed_events: usize,
}

/// Exercises the reusable local client contract against the real daemon IPC
/// boundary with deterministic storage and provider behavior.
#[allow(clippy::too_many_lines)]
pub async fn verify_local_client_contract() -> Result<ClientContractReport, ClientContractError> {
    let application = Application::in_memory(
        StartupConfiguration::safe_builtins()?,
        FakeBehavior {
            response_delay: Duration::from_mins(1),
            ..FakeBehavior::default()
        },
    )?;
    let harness = LocalDaemonHarness::start(application)?;
    let mut controller =
        ProtocolTestClient::connect(harness.endpoint(), "contract-controller").await?;
    let mut observer = ProtocolTestClient::connect(harness.endpoint(), "contract-observer").await?;
    let mut methods = BTreeSet::from(["initialize"]);

    let status = controller
        .call(command(None, Command::StatusGet(EmptyParams {})))
        .await?;
    decode::<StatusResult>(status)?;
    methods.insert("status.get");

    let created = controller
        .call(command(
            None,
            Command::SessionCreate(CreateSessionParams {
                persistent: true,
                configuration_overrides: None,
            }),
        ))
        .await?;
    let created = decode::<CreateSessionResult>(created)?;
    methods.insert("session.create");

    let listed = controller
        .call(command(
            None,
            Command::SessionList(ListSessionsParams {
                limit: 1,
                before_session_id: None,
            }),
        ))
        .await?;
    let listed = decode::<ListSessionsResult>(listed)?;
    if listed.sessions.len() != 1 || listed.sessions[0].session_id != created.session_id {
        return Err(ClientContractError::InvalidSessionList);
    }
    methods.insert("session.list");

    let prompt = controller
        .call(command(
            Some(created.session_id),
            Command::SessionPrompt(PromptParams {
                text: "exercise the reusable client contract".to_owned(),
                explicit_target: None,
            }),
        ))
        .await?;
    decode::<PromptResult>(prompt)?;
    methods.insert("session.prompt");

    let attached = observer
        .call(command(
            Some(created.session_id),
            Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
        ))
        .await?;
    let attached = decode::<AttachSessionResult>(attached)?;
    methods.insert("session.attach");

    let mut observed_events = 0;
    let mut approval_id = None;
    while observed_events < usize::try_from(attached.last_sequence).unwrap_or(usize::MAX) {
        let event = observer.next_event().await?;
        if event.kind == workbench_protocol::EventKind::ApprovalRequested {
            approval_id = event
                .data
                .get("approval_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok());
        }
        observed_events += 1;
    }
    let approved = controller
        .call(command(
            Some(created.session_id),
            Command::SessionApprovalResolve(ApprovalParams {
                approval_id: approval_id.ok_or(ClientContractError::MissingApproval)?,
                decision: ApprovalDecision::Grant,
            }),
        ))
        .await?;
    decode::<ApprovalResult>(approved)?;
    methods.insert("session.approval.resolve");

    let paused = controller
        .call(command(
            Some(created.session_id),
            Command::SessionPause(EmptyParams {}),
        ))
        .await?;
    decode::<ControlResult>(paused)?;
    methods.insert("session.pause");

    let redirected = controller
        .call(command(
            Some(created.session_id),
            Command::SessionRedirect(RedirectParams {
                instruction: "keep prior history intact".to_owned(),
            }),
        ))
        .await?;
    decode::<ControlResult>(redirected)?;
    methods.insert("session.redirect");

    let resumed = controller
        .call(command(
            Some(created.session_id),
            Command::SessionResume(EmptyParams {}),
        ))
        .await?;
    decode::<ControlResult>(resumed)?;
    methods.insert("session.resume");

    let session = controller
        .call(command(
            Some(created.session_id),
            Command::SessionGet(EmptyParams {}),
        ))
        .await?;
    decode::<SessionResult>(session)?;
    methods.insert("session.get");

    expect_protocol_error(
        controller
            .call(command(
                Some(created.session_id),
                Command::SessionReconcile(ReconciliationParams {
                    attempt_id: Uuid::now_v7(),
                    resolution: ReconciliationResolution::Retry,
                    evidence: None,
                }),
            ))
            .await,
        ErrorCode::InvalidTransition,
    )?;
    methods.insert("session.reconcile");

    let export_directory = tempfile::TempDir::new()?;
    let identity = age::x25519::Identity::generate();
    let exported = controller
        .call(command(
            Some(created.session_id),
            Command::SessionExport(ExportParams {
                output_path: export_directory
                    .path()
                    .join("session.age")
                    .to_string_lossy()
                    .into_owned(),
                age_recipients: vec![identity.to_public().to_string()],
            }),
        ))
        .await?;
    decode::<ExportResult>(exported)?;
    methods.insert("session.export");

    expect_protocol_error(
        controller
            .call(command(
                Some(created.session_id),
                Command::SessionDelete(DeleteParams {
                    confirm_session_id: created.session_id,
                }),
            ))
            .await,
        ErrorCode::InvalidTransition,
    )?;
    methods.insert("session.delete");

    let cancelled = controller
        .call(command(
            Some(created.session_id),
            Command::SessionCancel(EmptyParams {}),
        ))
        .await?;
    decode::<ControlResult>(cancelled)?;
    methods.insert("session.cancel");

    Ok(ClientContractReport {
        methods,
        observed_events,
    })
}

fn command(session_id: Option<Uuid>, command: Command) -> ClientCommand {
    ClientCommand {
        protocol: PROTOCOL_V1.to_owned(),
        request_id: Uuid::now_v7(),
        session_id,
        command,
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ClientContractError> {
    serde_json::from_value(value).map_err(ClientContractError::InvalidResult)
}

fn expect_protocol_error(
    result: Result<Value, TestClientError>,
    expected: ErrorCode,
) -> Result<(), ClientContractError> {
    match result {
        Err(TestClientError::Protocol(error))
            if error.code == expected
                && !error.retryable
                && !error.message.is_empty()
                && error.correlation_id.get_version_num() == 7 =>
        {
            Ok(())
        }
        Err(error) => Err(ClientContractError::Client(error)),
        Ok(_) => Err(ClientContractError::ExpectedProtocolError),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientContractError {
    #[error("daemon startup configuration failed")]
    Configuration(#[from] workbench_config::ConfigError),
    #[error("daemon storage fixture failed")]
    Storage(#[from] workbench_storage::StorageError),
    #[error("local daemon fixture failed")]
    Io(#[from] std::io::Error),
    #[error("local protocol client contract failed")]
    Client(#[from] TestClientError),
    #[error("method result violated its schema")]
    InvalidResult(serde_json::Error),
    #[error("invalid-state method unexpectedly succeeded")]
    ExpectedProtocolError,
    #[error("approval-required prompt emitted no approval identifier")]
    MissingApproval,
    #[error("session.list did not return the created session")]
    InvalidSessionList,
}
