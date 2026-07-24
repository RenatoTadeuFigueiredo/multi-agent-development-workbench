use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::Error as _,
    ser::{Error as _, SerializeMap},
};
use uuid::Uuid;

use crate::PROTOCOL_V1;

#[derive(Clone, Debug, PartialEq)]
pub enum ServerReply<T> {
    Success {
        request_id: Uuid,
        result: T,
    },
    Failure {
        request_id: Uuid,
        error: ProtocolError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub correlation_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    FrameTooLarge,
    UnauthorizedPeer,
    SessionNotFound,
    InvalidTransition,
    CapabilityUnavailable,
    PolicyDenied,
    ApprovalRequired,
    ProviderUnavailable,
    ProviderTimeout,
    OutcomeUnknown,
    ClientLagged,
    StorageUnavailable,
    KeyStoreUnavailable,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeResult {
    pub selected_protocol: ProtocolVersion,
    pub max_frame_bytes: u64,
    pub max_client_queue_events: u64,
    pub max_client_queue_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolVersion {
    #[serde(rename = "workbench/1")]
    V1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusResult {
    pub daemon_version: String,
    pub protocol: ProtocolVersion,
    pub storage_schema_version: u32,
    pub key_store: KeyStoreStatus,
    pub migration: MigrationStatus,
    pub active_sessions: u64,
    pub adapters: Vec<AdapterHealth>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStoreStatus {
    Available,
    Locked,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterHealth {
    pub id: String,
    pub status: AdapterStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterStatus {
    Available,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionResult {
    pub session_id: Uuid,
    pub configuration_hash: String,
    pub lock_hash: String,
    pub state: ReadyState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyState {
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionResult {
    pub session_id: Uuid,
    pub state: SessionState,
    pub last_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_approval_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncertain_attempt_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachSessionResult {
    pub session_id: Uuid,
    pub state: SessionState,
    pub replay_after_sequence: u64,
    pub last_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptResult {
    pub input_id: Uuid,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlResult {
    pub control_id: Uuid,
    pub control: Control,
    pub state: SessionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Pause,
    Resume,
    Redirect,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalResult {
    pub approval_id: Uuid,
    pub decision: crate::command::ApprovalDecision,
    pub state: SessionState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationResult {
    pub attempt_id: Uuid,
    pub resolution: crate::command::ReconciliationResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_attempt_id: Option<Uuid>,
    pub state: SessionState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportResult {
    pub export_id: Uuid,
    pub format: ExportFormat,
    pub recipient_fingerprints: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    #[serde(rename = "age-v1")]
    AgeV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteResult {
    pub deletion_id: Uuid,
    pub state: DeleteState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteState {
    Deleting,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Ready,
    Running,
    Pausing,
    Paused,
    AwaitingClarification,
    AwaitingApproval,
    CancelRequested,
    OutcomeUnknown,
    Completed,
    Failed,
    Cancelled,
    Abandoned,
    Deleting,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct RawReply<T> {
    protocol: String,
    request_id: Uuid,
    ok: bool,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    error: Option<ProtocolError>,
}

impl<'de, T> Deserialize<'de> for ServerReply<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawReply::deserialize(deserializer)?;
        if raw.protocol != PROTOCOL_V1 {
            return Err(D::Error::custom("unsupported protocol major"));
        }
        match (raw.ok, raw.result, raw.error) {
            (true, Some(result), None) => Ok(Self::Success {
                request_id: raw.request_id,
                result,
            }),
            (false, None, Some(error)) => {
                error.validate().map_err(D::Error::custom)?;
                Ok(Self::Failure {
                    request_id: raw.request_id,
                    error,
                })
            }
            _ => Err(D::Error::custom(
                "reply must contain exactly one valid outcome",
            )),
        }
    }
}

impl<T: Serialize> Serialize for ServerReply<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("protocol", PROTOCOL_V1)?;
        match self {
            Self::Success { request_id, result } => {
                map.serialize_entry("request_id", request_id)?;
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("result", result)?;
            }
            Self::Failure { request_id, error } => {
                error.validate().map_err(S::Error::custom)?;
                map.serialize_entry("request_id", request_id)?;
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("error", error)?;
            }
        }
        map.end()
    }
}

impl ProtocolError {
    fn validate(&self) -> Result<(), String> {
        if self.message.is_empty() || self.message.len() > 512 {
            return Err("error message must contain 1 to 512 bytes".to_owned());
        }
        if matches!(
            self.code,
            ErrorCode::InvalidRequest
                | ErrorCode::UnsupportedVersion
                | ErrorCode::FrameTooLarge
                | ErrorCode::UnauthorizedPeer
                | ErrorCode::SessionNotFound
                | ErrorCode::InvalidTransition
                | ErrorCode::CapabilityUnavailable
                | ErrorCode::PolicyDenied
                | ErrorCode::ApprovalRequired
                | ErrorCode::OutcomeUnknown
                | ErrorCode::ClientLagged
                | ErrorCode::KeyStoreUnavailable
                | ErrorCode::Internal
        ) && self.retryable
        {
            return Err("error category cannot be marked retryable".to_owned());
        }
        Ok(())
    }
}
