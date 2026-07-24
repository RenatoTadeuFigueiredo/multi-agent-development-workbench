use std::collections::HashSet;

use serde_json::{Value, json};

use crate::{ClaudeError, ClaudeErrorKind};

const MAX_IDENTIFIER_BYTES: usize = 4_096;
const MAX_TOOL_NAME_BYTES: usize = 64;
const MAX_CONTENT_BLOCKS: usize = 1_024;
const ALLOWED_TOOLS: [&str; 3] = ["Read", "Glob", "Grep"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Inbound {
    SystemInit {
        version: Option<String>,
    },
    ControlResponse {
        request_id: String,
        success: bool,
    },
    TextDelta(String),
    Assistant {
        text: Vec<String>,
        tools: Vec<String>,
    },
    ToolStarted(String),
    Result {
        is_error: bool,
        subtype: String,
        terminal_reason: Option<String>,
    },
    Ignored,
}

pub(crate) fn initialize_request(request_id: &str) -> Value {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {
            "subtype": "initialize",
            "hooks": null,
            "agents": {},
            "skills": []
        }
    })
}

pub(crate) fn interrupt_request(request_id: &str) -> Value {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {
            "subtype": "interrupt"
        }
    })
}

pub(crate) fn user_message(session_id: &str, text: &str) -> Value {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": text
        },
        "parent_tool_use_id": null,
        "session_id": session_id
    })
}

pub(crate) fn parse_inbound(value: &Value) -> Result<Inbound, ClaudeError> {
    let object = value.as_object().ok_or_else(protocol_violation)?;
    let kind = required_string(object.get("type"))?;
    match kind {
        "system" => parse_system(object),
        "control_response" => parse_control_response(object),
        "stream_event" => parse_stream_event(object),
        "assistant" => parse_assistant(object),
        "user" => parse_user(object),
        "result" => parse_result(object),
        "control_request" | "control_cancel_request" => Err(capability_violation()),
        _ => Err(protocol_violation()),
    }
}

fn parse_system(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    let subtype = required_string(object.get("subtype"))?;
    if subtype != "init" {
        return Ok(Inbound::Ignored);
    }
    optional_identifier(object.get("session_id"))?;
    let version = object
        .get("claude_code_version")
        .or_else(|| object.get("version"))
        .map(|value| required_string(Some(value)))
        .transpose()?
        .map(ToOwned::to_owned);
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(capability_violation)?;
    if tools.len() != ALLOWED_TOOLS.len() {
        return Err(capability_violation());
    }
    let mut advertised = HashSet::new();
    for tool in tools {
        let tool = validate_tool(required_string(Some(tool))?)?;
        if !advertised.insert(tool) {
            return Err(capability_violation());
        }
    }
    Ok(Inbound::SystemInit { version })
}

fn parse_control_response(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    let response = object
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    let request_id = required_identifier(response.get("request_id"))?.to_owned();
    let subtype = required_string(response.get("subtype"))?;
    if !matches!(subtype, "success" | "error") {
        return Err(protocol_violation());
    }
    Ok(Inbound::ControlResponse {
        request_id,
        success: subtype == "success",
    })
}

fn parse_stream_event(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    required_identifier(object.get("session_id"))?;
    required_identifier(object.get("uuid"))?;
    let event = object
        .get("event")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    match required_string(event.get("type"))? {
        "content_block_delta" => {
            let delta = event
                .get("delta")
                .and_then(Value::as_object)
                .ok_or_else(protocol_violation)?;
            match required_string(delta.get("type"))? {
                "text_delta" => Ok(Inbound::TextDelta(
                    required_string(delta.get("text"))?.to_owned(),
                )),
                "thinking_delta" | "signature_delta" | "input_json_delta" => Ok(Inbound::Ignored),
                _ => Err(capability_violation()),
            }
        }
        "content_block_start" => {
            let block = event
                .get("content_block")
                .and_then(Value::as_object)
                .ok_or_else(protocol_violation)?;
            match required_string(block.get("type"))? {
                "tool_use" => {
                    let name = validate_tool(required_string(block.get("name"))?)?;
                    Ok(Inbound::ToolStarted(name.to_owned()))
                }
                "text" | "thinking" | "redacted_thinking" => Ok(Inbound::Ignored),
                _ => Err(capability_violation()),
            }
        }
        "message_start" | "message_delta" | "message_stop" | "content_block_stop" => {
            Ok(Inbound::Ignored)
        }
        _ => Err(capability_violation()),
    }
}

fn parse_assistant(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    optional_identifier(object.get("session_id"))?;
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    if required_string(message.get("role"))? != "assistant" {
        return Err(protocol_violation());
    }
    let blocks = message
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(protocol_violation)?;
    if blocks.len() > MAX_CONTENT_BLOCKS {
        return Err(protocol_violation());
    }
    let mut text = Vec::new();
    let mut tools = Vec::new();
    for block in blocks {
        let block = block.as_object().ok_or_else(protocol_violation)?;
        match required_string(block.get("type"))? {
            "text" => {
                let value = required_string(block.get("text"))?;
                if !value.is_empty() {
                    text.push(value.to_owned());
                }
            }
            "tool_use" => {
                tools.push(validate_tool(required_string(block.get("name"))?)?.to_owned());
            }
            "thinking" | "redacted_thinking" => {}
            _ => return Err(capability_violation()),
        }
    }
    Ok(Inbound::Assistant { text, tools })
}

fn parse_user(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    optional_identifier(object.get("session_id"))?;
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    if required_string(message.get("role"))? != "user" {
        return Err(protocol_violation());
    }
    match message.get("content") {
        Some(Value::String(_)) => {}
        Some(Value::Array(blocks)) if blocks.len() <= MAX_CONTENT_BLOCKS => {
            for block in blocks {
                let block = block.as_object().ok_or_else(protocol_violation)?;
                if required_string(block.get("type"))? != "tool_result" {
                    return Err(capability_violation());
                }
                required_identifier(block.get("tool_use_id"))?;
            }
        }
        _ => return Err(protocol_violation()),
    }
    Ok(Inbound::Ignored)
}

fn parse_result(object: &serde_json::Map<String, Value>) -> Result<Inbound, ClaudeError> {
    optional_identifier(object.get("session_id"))?;
    let is_error = object
        .get("is_error")
        .and_then(Value::as_bool)
        .ok_or_else(protocol_violation)?;
    let subtype = required_identifier(object.get("subtype"))?.to_owned();
    let terminal_reason = object
        .get("terminal_reason")
        .filter(|value| !value.is_null())
        .map(|value| required_identifier(Some(value)))
        .transpose()?
        .map(ToOwned::to_owned);
    Ok(Inbound::Result {
        is_error,
        subtype,
        terminal_reason,
    })
}

fn validate_tool(name: &str) -> Result<&str, ClaudeError> {
    if name.is_empty()
        || name.len() > MAX_TOOL_NAME_BYTES
        || name.chars().any(char::is_control)
        || !ALLOWED_TOOLS.contains(&name)
    {
        Err(capability_violation())
    } else {
        Ok(name)
    }
}

fn required_identifier(value: Option<&Value>) -> Result<&str, ClaudeError> {
    let value = required_string(value)?;
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.chars().any(char::is_control)
    {
        Err(protocol_violation())
    } else {
        Ok(value)
    }
}

fn optional_identifier(value: Option<&Value>) -> Result<(), ClaudeError> {
    if value.is_none_or(Value::is_null) {
        Ok(())
    } else {
        required_identifier(value).map(|_| ())
    }
}

fn required_string(value: Option<&Value>) -> Result<&str, ClaudeError> {
    value.and_then(Value::as_str).ok_or_else(protocol_violation)
}

fn protocol_violation() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::ProtocolViolation,
        "Claude Code protocol is incompatible",
    )
}

fn capability_violation() -> ClaudeError {
    ClaudeError::new(
        ClaudeErrorKind::CapabilityUnavailable,
        "Claude Code requested an unavailable capability",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ClaudeErrorKind;

    use super::{Inbound, initialize_request, parse_inbound};

    #[test]
    fn initialization_has_no_hooks_agents_or_skills() {
        let request = initialize_request("init-1");
        assert_eq!(request["request"]["subtype"], "initialize");
        assert!(request["request"]["hooks"].is_null());
        assert_eq!(request["request"]["agents"], json!({}));
        assert_eq!(request["request"]["skills"], json!([]));
    }

    #[test]
    fn parses_text_and_discards_thinking() {
        let text = json!({
            "type": "stream_event",
            "uuid": "event-1",
            "session_id": "session-1",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "hello"}
            }
        });
        assert_eq!(
            parse_inbound(&text).expect("text"),
            Inbound::TextDelta("hello".to_owned())
        );
        let thinking = json!({
            "type": "stream_event",
            "uuid": "event-2",
            "session_id": "session-1",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "thinking_delta", "thinking": "private"}
            }
        });
        assert_eq!(
            parse_inbound(&thinking).expect("thinking"),
            Inbound::Ignored
        );
    }

    #[test]
    fn allows_only_the_three_read_tools() {
        for name in ["Read", "Glob", "Grep"] {
            let message = json!({
                "type": "assistant",
                "session_id": "session-1",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "tool_use", "name": name, "input": {"secret": true}}]
                }
            });
            assert!(matches!(
                parse_inbound(&message).expect("read tool"),
                Inbound::Assistant { .. }
            ));
        }
        let denied = json!({
            "type": "assistant",
            "session_id": "session-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "tool_use", "name": "Bash", "input": {}}]
            }
        });
        assert_eq!(
            parse_inbound(&denied).expect_err("write authority").kind(),
            ClaudeErrorKind::CapabilityUnavailable
        );
    }

    #[test]
    fn unknown_envelopes_and_child_control_requests_fail_closed() {
        for message in [
            json!({"type": "future_authority"}),
            json!({
                "type": "control_request",
                "request_id": "provider-request",
                "request": {"subtype": "can_use_tool", "tool_name": "Bash"}
            }),
        ] {
            assert!(parse_inbound(&message).is_err());
        }
    }

    #[test]
    fn initialization_and_nested_authority_fail_closed() {
        let valid_init = json!({
            "type": "system",
            "subtype": "init",
            "tools": ["Read", "Glob", "Grep"]
        });
        assert!(matches!(
            parse_inbound(&valid_init).expect("read-only init"),
            Inbound::SystemInit { .. }
        ));
        for message in [
            json!({
                "type": "system",
                "subtype": "init",
                "tools": ["Read", "Glob", "WebFetch"]
            }),
            json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "server_tool_use", "name": "web_search"}]
                }
            }),
            json!({
                "type": "stream_event",
                "uuid": "event-1",
                "session_id": "session-1",
                "event": {"type": "future_tool_event"}
            }),
        ] {
            assert!(parse_inbound(&message).is_err());
        }
    }
}
