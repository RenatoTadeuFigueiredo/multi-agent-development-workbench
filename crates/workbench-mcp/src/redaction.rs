//! Public tool event redaction helpers.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use workbench_core::policy::PolicySource;

use crate::error::McpErrorKind;

/// Bounded lifecycle categories exposed on the public protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolLifecycle {
    Planned,
    Allowed,
    Denied,
    ApprovalRequired,
    Approved,
    Started,
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

/// Redacted public tool event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicToolEvent {
    pub tool_name: String,
    pub lifecycle: ToolLifecycle,
    pub outcome: String,
    pub attempt_id: String,
    pub correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

impl PublicToolEvent {
    #[must_use]
    pub fn new(
        tool_name: impl Into<String>,
        lifecycle: ToolLifecycle,
        outcome: impl Into<String>,
        attempt_id: impl Into<String>,
        correlation_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            lifecycle,
            outcome: outcome.into(),
            attempt_id: attempt_id.into(),
            correlation_id: correlation_id.into(),
            policy_source: None,
            error_kind: None,
        }
    }

    #[must_use]
    pub fn with_policy_source(mut self, source: PolicySource) -> Self {
        self.policy_source = Some(policy_source_name(source).to_owned());
        self
    }

    #[must_use]
    pub fn with_error_kind(mut self, kind: McpErrorKind) -> Self {
        self.error_kind = Some(error_kind_name(kind).to_owned());
        self
    }

    /// Serializes without raw arguments, results, paths, or secrets.
    #[must_use]
    pub fn to_public_json(&self) -> Value {
        json!(self)
    }
}

const fn policy_source_name(source: PolicySource) -> &'static str {
    match source {
        PolicySource::BuiltIn => "built_in",
        PolicySource::User => "user",
        PolicySource::Repository => "repository",
        PolicySource::Session => "session",
        PolicySource::Role => "role",
        PolicySource::Workflow => "workflow",
        PolicySource::EffectClass => "effect_class",
    }
}

const fn error_kind_name(kind: McpErrorKind) -> &'static str {
    match kind {
        McpErrorKind::InvalidConfiguration => "invalid_configuration",
        McpErrorKind::PinMismatch => "pin_mismatch",
        McpErrorKind::Unavailable => "unavailable",
        McpErrorKind::PolicyDenied => "policy_denied",
        McpErrorKind::ApprovalRequired => "approval_required",
        McpErrorKind::ApprovalDenied => "approval_denied",
        McpErrorKind::TransportFailed => "transport_failed",
        McpErrorKind::ResponseTooLarge => "response_too_large",
        McpErrorKind::RedirectRejected => "redirect_rejected",
        McpErrorKind::Timeout => "timeout",
        McpErrorKind::Cancelled => "cancelled",
        McpErrorKind::OutcomeUnknown => "outcome_unknown",
        McpErrorKind::ShuttingDown => "shutting_down",
        McpErrorKind::ReapFailed => "reap_failed",
        McpErrorKind::Internal => "internal",
    }
}

/// Returns true when any forbidden marker appears in a public surface string.
#[must_use]
pub fn contains_marker(surface: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| surface.contains(marker))
}

/// Scrubs known sensitive object keys from a diagnostic value tree.
pub fn scrub_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if matches!(
                    key.as_str(),
                    "arguments"
                        | "result"
                        | "env"
                        | "headers"
                        | "executable"
                        | "url"
                        | "token"
                        | "password"
                        | "secret"
                        | "credential"
                        | "credential_ref"
                ) {
                    map.insert(key, Value::String("[redacted]".to_owned()));
                } else if let Some(child) = map.get_mut(&key) {
                    scrub_value(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_value(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_event_has_no_argument_fields() {
        let event = PublicToolEvent::new(
            "repo-read",
            ToolLifecycle::Succeeded,
            "ok",
            "attempt-1",
            "corr-1",
        );
        let encoded = event.to_public_json().to_string();
        assert!(!encoded.contains("arguments"));
        assert!(!encoded.contains("result"));
    }
}
