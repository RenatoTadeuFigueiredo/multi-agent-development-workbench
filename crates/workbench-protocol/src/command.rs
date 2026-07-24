use std::{
    collections::{BTreeSet, HashMap},
    path::{Component, Path},
};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::Error as _,
    ser::{Error as _, SerializeMap},
};
use serde_json::Value;
use uuid::{Uuid, Version};

use crate::PROTOCOL_V1;

#[derive(Clone, Debug, PartialEq)]
pub struct ClientCommand {
    pub protocol: String,
    pub request_id: Uuid,
    pub session_id: Option<Uuid>,
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Initialize(InitializeParams),
    StatusGet(EmptyParams),
    SessionCreate(CreateSessionParams),
    SessionList(ListSessionsParams),
    SessionGet(EmptyParams),
    SessionAttach(AttachSessionParams),
    SessionPrompt(PromptParams),
    SessionPause(EmptyParams),
    SessionResume(EmptyParams),
    SessionRedirect(RedirectParams),
    SessionCancel(EmptyParams),
    SessionApprovalResolve(ApprovalParams),
    SessionReconcile(ReconciliationParams),
    SessionExport(ExportParams),
    SessionDelete(DeleteParams),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeParams {
    pub client_name: String,
    pub client_version: String,
    pub supported_protocols: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionParams {
    pub persistent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_overrides: Option<HashMap<String, Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSessionsParams {
    #[serde(default = "default_session_list_limit")]
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_session_id: Option<Uuid>,
}

const fn default_session_list_limit() -> u16 {
    50
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachSessionParams {
    pub after_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptParams {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_target: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedirectParams {
    pub instruction: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalParams {
    pub approval_id: Uuid,
    pub decision: ApprovalDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Grant,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationParams {
    pub attempt_id: Uuid,
    pub resolution: ReconciliationResolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationResolution {
    Retry,
    AcceptResult,
    Abandon,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    pub output_path: String,
    pub age_recipients: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    pub confirm_session_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    protocol: String,
    request_id: Uuid,
    method: String,
    #[serde(default)]
    session_id: Option<Uuid>,
    params: Value,
}

impl<'de> Deserialize<'de> for ClientCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawCommand::deserialize(deserializer)?;
        if raw.protocol != PROTOCOL_V1 {
            return Err(D::Error::custom("unsupported protocol major"));
        }
        require_uuid_v7(raw.request_id, "request_id").map_err(D::Error::custom)?;
        if let Some(session_id) = raw.session_id {
            require_uuid_v7(session_id, "session_id").map_err(D::Error::custom)?;
        }
        let command = parse_command::<D::Error>(&raw.method, raw.params)?;
        let session_required = command.requires_session();
        if session_required != raw.session_id.is_some() {
            return Err(D::Error::custom(if session_required {
                "session_id is required for this method"
            } else {
                "session_id is forbidden for this method"
            }));
        }
        command.validate().map_err(D::Error::custom)?;
        Ok(Self {
            protocol: raw.protocol,
            request_id: raw.request_id,
            session_id: raw.session_id,
            command,
        })
    }
}

impl Serialize for ClientCommand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (method, params) = self.command.wire_parts().map_err(S::Error::custom)?;
        let mut map =
            serializer.serialize_map(Some(if self.session_id.is_some() { 5 } else { 4 }))?;
        map.serialize_entry("protocol", &self.protocol)?;
        map.serialize_entry("request_id", &self.request_id)?;
        map.serialize_entry("method", method)?;
        if let Some(session_id) = self.session_id {
            map.serialize_entry("session_id", &session_id)?;
        }
        map.serialize_entry("params", &params)?;
        map.end()
    }
}

impl Command {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Initialize(_) => "initialize",
            Self::StatusGet(_) => "status.get",
            Self::SessionCreate(_) => "session.create",
            Self::SessionList(_) => "session.list",
            Self::SessionGet(_) => "session.get",
            Self::SessionAttach(_) => "session.attach",
            Self::SessionPrompt(_) => "session.prompt",
            Self::SessionPause(_) => "session.pause",
            Self::SessionResume(_) => "session.resume",
            Self::SessionRedirect(_) => "session.redirect",
            Self::SessionCancel(_) => "session.cancel",
            Self::SessionApprovalResolve(_) => "session.approval.resolve",
            Self::SessionReconcile(_) => "session.reconcile",
            Self::SessionExport(_) => "session.export",
            Self::SessionDelete(_) => "session.delete",
        }
    }

    pub fn requires_session(&self) -> bool {
        !matches!(
            self,
            Self::Initialize(_)
                | Self::StatusGet(_)
                | Self::SessionCreate(_)
                | Self::SessionList(_)
        )
    }

    fn wire_parts(&self) -> Result<(&'static str, Value), serde_json::Error> {
        let value = match self {
            Self::Initialize(value) => serde_json::to_value(value)?,
            Self::StatusGet(value)
            | Self::SessionGet(value)
            | Self::SessionPause(value)
            | Self::SessionResume(value)
            | Self::SessionCancel(value) => serde_json::to_value(value)?,
            Self::SessionCreate(value) => serde_json::to_value(value)?,
            Self::SessionList(value) => serde_json::to_value(value)?,
            Self::SessionAttach(value) => serde_json::to_value(value)?,
            Self::SessionPrompt(value) => serde_json::to_value(value)?,
            Self::SessionRedirect(value) => serde_json::to_value(value)?,
            Self::SessionApprovalResolve(value) => serde_json::to_value(value)?,
            Self::SessionReconcile(value) => serde_json::to_value(value)?,
            Self::SessionExport(value) => serde_json::to_value(value)?,
            Self::SessionDelete(value) => serde_json::to_value(value)?,
        };
        Ok((self.method(), value))
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Initialize(params) => {
                if params.client_name.is_empty()
                    || params.client_name.len() > 128
                    || params.client_version.is_empty()
                    || params.client_version.len() > 128
                {
                    return Err("client name and version must contain 1 to 128 bytes".to_owned());
                }
                let unique = params.supported_protocols.iter().collect::<BTreeSet<_>>();
                if params.supported_protocols.is_empty()
                    || unique.len() != params.supported_protocols.len()
                    || params
                        .supported_protocols
                        .iter()
                        .any(|protocol| protocol.len() > 32)
                {
                    return Err("supported_protocols must be non-empty and unique".to_owned());
                }
            }
            Self::SessionCreate(params) if !params.persistent => {
                return Err("feature 001 supports persistent sessions only".to_owned());
            }
            Self::SessionList(params) => {
                if !(1..=100).contains(&params.limit) {
                    return Err("limit must be between 1 and 100".to_owned());
                }
                if let Some(before_session_id) = params.before_session_id {
                    require_uuid_v7(before_session_id, "before_session_id")?;
                }
            }
            Self::SessionPrompt(params) => {
                validate_content(&params.text, "text")?;
                if let Some(target) = &params.explicit_target {
                    validate_identifier(target)?;
                }
            }
            Self::SessionRedirect(params) => validate_content(&params.instruction, "instruction")?,
            Self::SessionApprovalResolve(params) => {
                require_uuid_v7(params.approval_id, "approval_id")?;
            }
            Self::SessionReconcile(params) => {
                require_uuid_v7(params.attempt_id, "attempt_id")?;
                if params
                    .evidence
                    .as_ref()
                    .is_some_and(|value| value.len() > 4_096)
                {
                    return Err("evidence exceeds 4096 bytes".to_owned());
                }
            }
            Self::SessionExport(params) => {
                validate_output_path(&params.output_path)?;
                let unique = params.age_recipients.iter().collect::<BTreeSet<_>>();
                if params.age_recipients.is_empty()
                    || unique.len() != params.age_recipients.len()
                    || params
                        .age_recipients
                        .iter()
                        .any(|recipient| recipient.is_empty() || recipient.len() > 512)
                {
                    return Err("age_recipients must be non-empty, valid, and unique".to_owned());
                }
            }
            Self::SessionDelete(params) => {
                require_uuid_v7(params.confirm_session_id, "confirm_session_id")?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn parse_command<E: serde::de::Error>(method: &str, params: Value) -> Result<Command, E> {
    macro_rules! parse {
        ($variant:ident, $type:ty) => {
            serde_json::from_value::<$type>(params)
                .map(Command::$variant)
                .map_err(E::custom)
        };
    }
    match method {
        "initialize" => parse!(Initialize, InitializeParams),
        "status.get" => parse!(StatusGet, EmptyParams),
        "session.create" => parse!(SessionCreate, CreateSessionParams),
        "session.list" => parse!(SessionList, ListSessionsParams),
        "session.get" => parse!(SessionGet, EmptyParams),
        "session.attach" => parse!(SessionAttach, AttachSessionParams),
        "session.prompt" => parse!(SessionPrompt, PromptParams),
        "session.pause" => parse!(SessionPause, EmptyParams),
        "session.resume" => parse!(SessionResume, EmptyParams),
        "session.redirect" => parse!(SessionRedirect, RedirectParams),
        "session.cancel" => parse!(SessionCancel, EmptyParams),
        "session.approval.resolve" => parse!(SessionApprovalResolve, ApprovalParams),
        "session.reconcile" => parse!(SessionReconcile, ReconciliationParams),
        "session.export" => parse!(SessionExport, ExportParams),
        "session.delete" => parse!(SessionDelete, DeleteParams),
        _ => Err(E::custom("unknown protocol method")),
    }
}

fn require_uuid_v7(value: Uuid, field: &str) -> Result<(), String> {
    if value.get_version() == Some(Version::SortRand) {
        Ok(())
    } else {
        Err(format!("{field} must be a UUIDv7"))
    }
}

fn validate_content(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1_048_576 {
        Err(format!("{field} must contain 1 to 1048576 bytes"))
    } else {
        Ok(())
    }
}

fn validate_identifier(value: &str) -> Result<(), String> {
    let mut chars = value.chars();
    if chars
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && value.len() <= 63
    {
        Ok(())
    } else {
        Err("explicit_target has an invalid identifier".to_owned())
    }
}

fn validate_output_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.len() > 4_096
        || !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        Err("output_path must be an absolute non-traversing path".to_owned())
    } else {
        Ok(())
    }
}
