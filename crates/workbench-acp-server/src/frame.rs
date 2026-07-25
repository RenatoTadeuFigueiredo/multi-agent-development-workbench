use serde_json::Value;

use crate::{AcpServerError, AcpServerErrorKind};

/// Maximum encoded frame size excluding the newline delimiter.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Decodes one NDJSON line into a JSON value.
///
/// # Errors
///
/// Returns when the line is empty, oversized, non-UTF-8, or invalid JSON.
pub fn decode_line(line: &[u8]) -> Result<Value, AcpServerError> {
    if line.is_empty() {
        return Err(AcpServerError::new(
            AcpServerErrorKind::MalformedFrame,
            "ACP frame is empty",
        ));
    }
    if line.len() > MAX_FRAME_BYTES {
        return Err(AcpServerError::new(
            AcpServerErrorKind::FrameTooLarge,
            "ACP frame exceeds the encoded size ceiling",
        ));
    }
    let text = std::str::from_utf8(line).map_err(|_| {
        AcpServerError::new(
            AcpServerErrorKind::MalformedFrame,
            "ACP frame is not valid UTF-8",
        )
    })?;
    serde_json::from_str(text).map_err(|_| {
        AcpServerError::new(
            AcpServerErrorKind::MalformedFrame,
            "ACP frame is not valid JSON",
        )
    })
}

/// Encodes a JSON value as one NDJSON line without a trailing newline.
///
/// # Errors
///
/// Returns when the encoded payload exceeds the frame ceiling.
pub fn encode_message(value: &Value) -> Result<Vec<u8>, AcpServerError> {
    let bytes = serde_json::to_vec(value).map_err(|_| {
        AcpServerError::new(
            AcpServerErrorKind::InvalidRequest,
            "failed to encode ACP message",
        )
    })?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(AcpServerError::new(
            AcpServerErrorKind::FrameTooLarge,
            "ACP frame exceeds the encoded size ceiling",
        ));
    }
    Ok(bytes)
}
