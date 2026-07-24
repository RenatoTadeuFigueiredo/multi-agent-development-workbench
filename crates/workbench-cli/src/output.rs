use serde_json::{Value, json};
use uuid::Uuid;
use workbench_protocol::{ErrorCode, SessionEvent};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    InvalidInput = 2,
    PolicyRefusal = 3,
    UnavailableCapability = 4,
    StorageFailure = 5,
    OutcomeUnknown = 6,
    ProtocolFailure = 7,
    Internal = 70,
}

impl ExitCode {
    #[must_use]
    pub const fn from_protocol(code: ErrorCode) -> Self {
        match code {
            ErrorCode::InvalidRequest
            | ErrorCode::SessionNotFound
            | ErrorCode::InvalidTransition => Self::InvalidInput,
            ErrorCode::PolicyDenied | ErrorCode::ApprovalRequired => Self::PolicyRefusal,
            ErrorCode::CapabilityUnavailable
            | ErrorCode::ProviderUnavailable
            | ErrorCode::ProviderTimeout => Self::UnavailableCapability,
            ErrorCode::StorageUnavailable | ErrorCode::KeyStoreUnavailable => Self::StorageFailure,
            ErrorCode::OutcomeUnknown => Self::OutcomeUnknown,
            ErrorCode::UnsupportedVersion
            | ErrorCode::FrameTooLarge
            | ErrorCode::UnauthorizedPeer
            | ErrorCode::ClientLagged => Self::ProtocolFailure,
            ErrorCode::Internal => Self::Internal,
        }
    }
}

#[must_use]
pub fn success(request_id: Uuid, result: &Value) -> Value {
    json!({
        "schema_version": 1,
        "request_id": request_id,
        "ok": true,
        "result": result,
    })
}

#[must_use]
pub fn failure(request_id: Uuid, error: &Value) -> Value {
    json!({
        "schema_version": 1,
        "request_id": request_id,
        "ok": false,
        "error": error,
    })
}

#[must_use]
pub fn event(event: &SessionEvent) -> Value {
    json!({
        "schema_version": 1,
        "event_id": event.event_id,
        "session_id": event.session_id,
        "sequence": event.sequence,
        "event": event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_result_has_stable_envelope() {
        let request_id = Uuid::now_v7();
        let output = success(request_id, &json!({"state": "ready"}));

        assert_eq!(output["schema_version"], 1);
        assert_eq!(output["request_id"], request_id.to_string());
        assert_eq!(output["ok"], true);
        assert!(output.get("error").is_none());
    }

    #[test]
    fn json_failure_has_stable_envelope() {
        let request_id = Uuid::now_v7();
        let output = failure(request_id, &json!({"code": "invalid_request"}));

        assert_eq!(output["schema_version"], 1);
        assert_eq!(output["request_id"], request_id.to_string());
        assert_eq!(output["ok"], false);
        assert!(output.get("result").is_none());
    }

    #[test]
    fn stable_exit_codes_match_the_cli_contract() {
        assert_eq!(ExitCode::Success as i32, 0);
        assert_eq!(ExitCode::InvalidInput as i32, 2);
        assert_eq!(ExitCode::PolicyRefusal as i32, 3);
        assert_eq!(ExitCode::UnavailableCapability as i32, 4);
        assert_eq!(ExitCode::StorageFailure as i32, 5);
        assert_eq!(ExitCode::OutcomeUnknown as i32, 6);
        assert_eq!(ExitCode::ProtocolFailure as i32, 7);
        assert_eq!(ExitCode::Internal as i32, 70);
    }

    #[test]
    fn protocol_errors_map_to_stable_exit_categories() {
        assert_eq!(
            ExitCode::from_protocol(ErrorCode::PolicyDenied),
            ExitCode::PolicyRefusal
        );
        assert_eq!(
            ExitCode::from_protocol(ErrorCode::ProviderUnavailable),
            ExitCode::UnavailableCapability
        );
        assert_eq!(
            ExitCode::from_protocol(ErrorCode::KeyStoreUnavailable),
            ExitCode::StorageFailure
        );
        assert_eq!(
            ExitCode::from_protocol(ErrorCode::ClientLagged),
            ExitCode::ProtocolFailure
        );
    }
}
