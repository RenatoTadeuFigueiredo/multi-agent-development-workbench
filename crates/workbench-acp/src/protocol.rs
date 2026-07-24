use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::{
    AcpError, AcpErrorKind,
    transport::{Connection, PromptState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationStatus {
    Available,
    InteractiveRequired,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterHealth {
    Starting,
    Available,
    AuthenticationRequired,
    Incompatible,
    Unavailable,
    Crashed,
    ShuttingDown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpCapabilities {
    pub load_session: bool,
    pub authentication: AuthenticationStatus,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcpSession {
    pub(crate) id: String,
}

impl AcpSession {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    Acknowledged,
    AgentMessage,
    AgentThought,
    ToolCall,
    ToolCallUpdate,
    Plan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedUpdate {
    pub kind: UpdateKind,
    pub content: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptOutcome {
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationOutcome {
    Confirmed,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromptEvent {
    Update(NormalizedUpdate),
    Finished(PromptOutcome),
}

pub(crate) fn validate_jsonrpc(value: &Value) -> Result<(), AcpError> {
    let Some(object) = value.as_object() else {
        return Err(protocol_violation());
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(protocol_violation());
    }
    Ok(())
}

pub(crate) fn validate_jsonrpc_id(value: &Value) -> Result<(), AcpError> {
    if value.as_str().is_some() || value.as_i64().is_some() || value.as_u64().is_some() {
        Ok(())
    } else {
        Err(protocol_violation())
    }
}

pub(crate) fn parse_initialize(
    value: &Value,
) -> Result<(AcpCapabilities, Option<String>), AcpError> {
    let object = value.as_object().ok_or_else(protocol_violation)?;
    if object.get("protocolVersion").and_then(Value::as_u64) != Some(crate::ACP_PROTOCOL_VERSION) {
        return Err(AcpError::new(
            AcpErrorKind::IncompatibleProtocol,
            "ACP protocol version is incompatible",
        ));
    }
    let capabilities = object
        .get("agentCapabilities")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    if capabilities.get("loadSession").and_then(Value::as_bool) != Some(true) {
        return Err(AcpError::new(
            AcpErrorKind::CapabilityUnavailable,
            "required ACP capability is unavailable",
        ));
    }
    let auth_methods = object.get("authMethods").map_or(Ok(&[][..]), |value| {
        value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(protocol_violation)
    })?;
    let method_ids = auth_methods
        .iter()
        .map(|method| {
            bounded_string(
                method
                    .as_object()
                    .and_then(|object| object.get("id"))
                    .ok_or_else(protocol_violation)?,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let default_auth_method = object
        .get("_meta")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("defaultAuthMethodId"))
        .filter(|value| !value.is_null())
        .map(bounded_string)
        .transpose()?;
    if let Some(default) = default_auth_method.as_deref()
        && !method_ids.iter().any(|method| method == default)
    {
        return Err(protocol_violation());
    }
    let authentication = if default_auth_method.is_some() || auth_methods.is_empty() {
        AuthenticationStatus::Available
    } else {
        AuthenticationStatus::InteractiveRequired
    };
    let agent_info = object
        .get("agentInfo")
        .map(|value| value.as_object().ok_or_else(protocol_violation))
        .transpose()?;
    let agent_name = agent_info
        .and_then(|info| info.get("name"))
        .map(bounded_string)
        .transpose()?;
    let agent_version = agent_info
        .and_then(|info| info.get("version"))
        .map(bounded_string)
        .transpose()?;
    Ok((
        AcpCapabilities {
            load_session: true,
            authentication,
            agent_name,
            agent_version,
        },
        default_auth_method,
    ))
}

pub(crate) fn parse_session(value: &Value) -> Result<AcpSession, AcpError> {
    let session_id = value
        .as_object()
        .and_then(|object| object.get("sessionId"))
        .ok_or_else(protocol_violation)?;
    session_from_id(&bounded_string(session_id)?)
}

pub(crate) fn session_from_id(session_id: &str) -> Result<AcpSession, AcpError> {
    if session_id.is_empty() || session_id.len() > 4_096 {
        return Err(protocol_violation());
    }
    Ok(AcpSession {
        id: session_id.to_owned(),
    })
}

pub(crate) fn parse_prompt_outcome(value: &Value) -> Result<PromptOutcome, AcpError> {
    let stop_reason = value
        .as_object()
        .and_then(|object| object.get("stopReason"))
        .and_then(Value::as_str)
        .ok_or_else(protocol_violation)?;
    let stop_reason = match stop_reason {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "max_turn_requests" => StopReason::MaxTurnRequests,
        "refusal" => StopReason::Refusal,
        "cancelled" => StopReason::Cancelled,
        _ => return Err(protocol_violation()),
    };
    Ok(PromptOutcome { stop_reason })
}

pub(crate) fn parse_session_update(
    value: &Value,
) -> Result<(String, Option<NormalizedUpdate>), AcpError> {
    let object = value.as_object().ok_or_else(protocol_violation)?;
    let session_id = bounded_string(object.get("sessionId").ok_or_else(protocol_violation)?)?;
    session_from_id(&session_id)?;
    let update = object
        .get("update")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .ok_or_else(protocol_violation)?;
    let normalized = match kind {
        "agent_message_chunk" => Some(NormalizedUpdate {
            kind: UpdateKind::AgentMessage,
            content: Some(
                update
                    .get("content")
                    .cloned()
                    .ok_or_else(protocol_violation)?,
            ),
        }),
        "agent_thought_chunk" => Some(NormalizedUpdate {
            kind: UpdateKind::AgentThought,
            content: Some(
                update
                    .get("content")
                    .cloned()
                    .ok_or_else(protocol_violation)?,
            ),
        }),
        "tool_call" => Some(NormalizedUpdate {
            kind: UpdateKind::ToolCall,
            content: Some(Value::Object(update.clone())),
        }),
        "tool_call_update" => Some(NormalizedUpdate {
            kind: UpdateKind::ToolCallUpdate,
            content: Some(Value::Object(update.clone())),
        }),
        "plan" => Some(NormalizedUpdate {
            kind: UpdateKind::Plan,
            content: Some(Value::Object(update.clone())),
        }),
        _ => None,
    };
    Ok((session_id, normalized))
}

pub(crate) fn acknowledged() -> NormalizedUpdate {
    NormalizedUpdate {
        kind: UpdateKind::Acknowledged,
        content: None,
    }
}

fn bounded_string(value: &Value) -> Result<String, AcpError> {
    let value = value.as_str().ok_or_else(protocol_violation)?;
    if value.is_empty() || value.len() > 4_096 {
        return Err(protocol_violation());
    }
    Ok(value.to_owned())
}

fn protocol_violation() -> AcpError {
    AcpError::new(
        AcpErrorKind::ProtocolViolation,
        "ACP peer violated the protocol",
    )
}

pub struct PromptExecution {
    updates: mpsc::Receiver<NormalizedUpdate>,
    state: watch::Receiver<PromptState>,
    control: PromptControl,
    finished: bool,
}

impl PromptExecution {
    pub(crate) fn new(
        updates: mpsc::Receiver<NormalizedUpdate>,
        state: watch::Receiver<PromptState>,
        control: PromptControl,
    ) -> Self {
        Self {
            updates,
            state,
            control,
            finished: false,
        }
    }

    #[must_use]
    pub fn control(&self) -> PromptControl {
        self.control.clone()
    }

    /// Returns the next normalized update or the single terminal outcome.
    ///
    /// # Errors
    ///
    /// Returns a redacted transport or protocol error when the child fails
    /// before a definite terminal prompt response is received.
    pub async fn next(&mut self) -> Result<Option<PromptEvent>, AcpError> {
        if self.finished {
            return Ok(None);
        }
        loop {
            if let PromptState::Terminal(result) = self.state.borrow().clone() {
                if let Ok(update) = self.updates.try_recv() {
                    return Ok(Some(PromptEvent::Update(update)));
                }
                self.finished = true;
                return result.map(|outcome| Some(PromptEvent::Finished(outcome)));
            }
            tokio::select! {
                update = self.updates.recv() => {
                    if let Some(update) = update {
                        return Ok(Some(PromptEvent::Update(update)));
                    }
                    if self.state.changed().await.is_err() {
                        return Err(AcpError::new(
                            AcpErrorKind::TransportClosed,
                            "ACP prompt state is unavailable",
                        ));
                    }
                }
                changed = self.state.changed() => {
                    if changed.is_err() {
                        return Err(AcpError::new(
                            AcpErrorKind::TransportClosed,
                            "ACP prompt state is unavailable",
                        ));
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct PromptControl {
    pub(crate) connection: Weak<Connection>,
    pub(crate) session_id: Arc<str>,
    pub(crate) state: watch::Receiver<PromptState>,
    pub(crate) cancel_sent: Arc<AtomicBool>,
}

impl PromptControl {
    pub async fn cancel(&self, deadline: Duration) -> CancellationOutcome {
        let mut state = self.state.clone();
        let cancellation = async {
            if !self.cancel_sent.swap(true, Ordering::AcqRel) {
                let Some(connection) = self.connection.upgrade() else {
                    return Err(AcpError::new(
                        AcpErrorKind::TransportClosed,
                        "ACP transport is unavailable",
                    ));
                };
                connection.cancel(&self.session_id).await?;
            }
            loop {
                if let PromptState::Terminal(result) = state.borrow().clone() {
                    return result;
                }
                if state.changed().await.is_err() {
                    return Err(AcpError::new(
                        AcpErrorKind::TransportClosed,
                        "ACP prompt state is unavailable",
                    ));
                }
            }
        };
        match tokio::time::timeout(deadline, cancellation).await {
            Ok(Ok(PromptOutcome {
                stop_reason: StopReason::Cancelled,
            })) => CancellationOutcome::Confirmed,
            _ => CancellationOutcome::Unconfirmed,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_initialize;

    #[test]
    fn optional_agent_info_may_be_omitted_but_not_malformed() {
        let baseline = json!({
            "protocolVersion": 1,
            "agentCapabilities": {"loadSession": true},
            "authMethods": []
        });
        let (capabilities, _) = parse_initialize(&baseline).expect("omitted agent info");
        assert_eq!(capabilities.agent_version, None);

        let mut malformed = baseline;
        malformed["agentInfo"] = json!("invalid");
        assert!(parse_initialize(&malformed).is_err());
    }
}
