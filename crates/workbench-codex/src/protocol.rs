use serde_json::Value;

use crate::{CodexError, CodexErrorKind};

const MAX_IDENTIFIER_BYTES: usize = 4_096;
const MAX_TOOL_NAME_BYTES: usize = 64;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Allowed bounded tool item names for read-only observation.
const ALLOWED_TOOL_ITEM_TYPES: [&str; 3] = ["command_execution", "web_search", "search"];

/// Item types that prove elevated authority and fail closed.
const FORBIDDEN_ITEM_TYPES: [&str; 5] = [
    "file_change",
    "mcp_tool_call",
    "mcp_tool_call_begin",
    "mcp_tool_call_end",
    "computer_use",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Inbound {
    ThreadStarted,
    TurnStarted,
    Text {
        text: String,
    },
    ToolStarted {
        name: String,
    },
    TurnCompleted,
    TurnFailed {
        cancelled: bool,
    },
    Error {
        cancelled: bool,
    },
    Ignored,
}

pub(crate) fn parse_inbound(value: &Value) -> Result<Inbound, CodexError> {
    let object = value.as_object().ok_or_else(protocol_violation)?;
    let kind = required_string(object.get("type"))?;
    match kind {
        "thread.started" => {
            optional_identifier(object.get("thread_id"))?;
            Ok(Inbound::ThreadStarted)
        }
        "turn.started" => Ok(Inbound::TurnStarted),
        "turn.completed" => {
            // Usage bodies are intentionally unread.
            let _ = object.get("usage");
            Ok(Inbound::TurnCompleted)
        }
        "turn.failed" => Ok(Inbound::TurnFailed {
            cancelled: cancelled_marker(object.get("error"))
                || cancelled_string(object.get("reason")),
        }),
        "item.started" | "item.completed" | "item.updated" => parse_item(kind, object),
        "error" => Ok(Inbound::Error {
            cancelled: cancelled_marker(Some(value))
                || cancelled_string(object.get("message"))
                || cancelled_string(object.get("code")),
        }),
        // Additive unknown event types never grant authority or terminal success.
        _ => Ok(Inbound::Ignored),
    }
}

fn parse_item(
    kind: &str,
    object: &serde_json::Map<String, Value>,
) -> Result<Inbound, CodexError> {
    let item = object
        .get("item")
        .and_then(Value::as_object)
        .ok_or_else(protocol_violation)?;
    optional_identifier(item.get("id"))?;
    let item_type = required_string(item.get("type"))?;
    if item_type.is_empty()
        || item_type.len() > MAX_TOOL_NAME_BYTES
        || item_type.chars().any(char::is_control)
    {
        return Err(protocol_violation());
    }
    if FORBIDDEN_ITEM_TYPES.contains(&item_type) {
        return Err(capability_violation());
    }
    match item_type {
        "agent_message" => {
            if kind == "item.completed" {
                let text = required_string(item.get("text"))?;
                if text.is_empty() {
                    return Ok(Inbound::Ignored);
                }
                if text.len() > MAX_TEXT_BYTES || text.chars().any(|ch| ch == '\0') {
                    return Err(protocol_violation());
                }
                Ok(Inbound::Text {
                    text: text.to_owned(),
                })
            } else {
                Ok(Inbound::Ignored)
            }
        }
        "reasoning" | "plan_update" | "todo_list" => Ok(Inbound::Ignored),
        name if ALLOWED_TOOL_ITEM_TYPES.contains(&name) => {
            if kind == "item.started" {
                Ok(Inbound::ToolStarted {
                    name: name.to_owned(),
                })
            } else {
                Ok(Inbound::Ignored)
            }
        }
        _ => {
            // Unknown item types do not grant authority; ignore after validation.
            Ok(Inbound::Ignored)
        }
    }
}

fn cancelled_marker(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if let Some(object) = value.as_object() {
        return cancelled_string(object.get("type"))
            || cancelled_string(object.get("code"))
            || cancelled_string(object.get("message"))
            || cancelled_string(object.get("reason"));
    }
    cancelled_string(Some(value))
}

fn cancelled_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| {
            let lowered = text.to_ascii_lowercase();
            lowered.contains("cancel") || lowered.contains("abort")
        })
}

fn optional_identifier(value: Option<&Value>) -> Result<(), CodexError> {
    if value.is_none_or(Value::is_null) {
        Ok(())
    } else {
        required_identifier(value).map(|_| ())
    }
}

fn required_identifier(value: Option<&Value>) -> Result<&str, CodexError> {
    let value = required_string(value)?;
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.chars().any(char::is_control)
    {
        Err(protocol_violation())
    } else {
        Ok(value)
    }
}

fn required_string(value: Option<&Value>) -> Result<&str, CodexError> {
    value.and_then(Value::as_str).ok_or_else(protocol_violation)
}

fn protocol_violation() -> CodexError {
    CodexError::new(
        CodexErrorKind::ProtocolViolation,
        "Codex protocol is incompatible",
    )
}

fn capability_violation() -> CodexError {
    CodexError::new(
        CodexErrorKind::CapabilityUnavailable,
        "Codex requested an unavailable capability",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::CodexErrorKind;

    use super::{Inbound, parse_inbound};

    #[test]
    fn parses_text_and_ignores_reasoning() {
        let text = json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "agent_message",
                "text": "hello"
            }
        });
        assert_eq!(
            parse_inbound(&text).expect("text"),
            Inbound::Text {
                text: "hello".to_owned()
            }
        );
        let reasoning = json!({
            "type": "item.completed",
            "item": {
                "id": "item_2",
                "type": "reasoning",
                "text": "private"
            }
        });
        assert_eq!(
            parse_inbound(&reasoning).expect("reasoning"),
            Inbound::Ignored
        );
    }

    #[test]
    fn rejects_file_change_items() {
        let file_change = json!({
            "type": "item.completed",
            "item": {
                "id": "item_3",
                "type": "file_change",
                "changes": []
            }
        });
        assert_eq!(
            parse_inbound(&file_change)
                .expect_err("file change")
                .kind(),
            CodexErrorKind::CapabilityUnavailable
        );
    }

    #[test]
    fn detects_cancelled_turn_failure() {
        let failed = json!({
            "type": "turn.failed",
            "error": {"type": "cancelled", "message": "aborted"}
        });
        assert_eq!(
            parse_inbound(&failed).expect("failed"),
            Inbound::TurnFailed { cancelled: true }
        );
    }

    #[test]
    fn unknown_event_types_are_ignored() {
        assert_eq!(
            parse_inbound(&json!({"type": "future.event", "extra": 1})).expect("future"),
            Inbound::Ignored
        );
    }
}
