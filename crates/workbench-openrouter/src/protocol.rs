use serde_json::Value;
use workbench_core::{ports::ProviderOutput, value::NonEmptyText};

use crate::{OpenRouterError, OpenRouterErrorKind};

/// Parsed usage and spend from a terminal completion payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSummary {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd_micros: u64,
}

/// Normalize one SSE data payload into zero or more provider outputs.
///
/// # Errors
///
/// Returns when the JSON is malformed.
pub fn normalize_sse_data(data: &str) -> Result<Vec<ProviderOutput>, OpenRouterError> {
    if data.trim() == "[DONE]" {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(data).map_err(|_| {
        OpenRouterError::new(
            OpenRouterErrorKind::MalformedStream,
            "OpenRouter stream frame is not valid JSON",
        )
    })?;
    let mut outputs = Vec::new();
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("OpenRouter returned an error");
        return Err(OpenRouterError::new(
            OpenRouterErrorKind::Unavailable,
            message,
        ));
    }
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OpenRouterError::new(
                OpenRouterErrorKind::MalformedStream,
                "OpenRouter stream frame is missing choices",
            )
        })?;
    for choice in choices {
        if let Some(delta) = choice.get("delta")
            && let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            let content = NonEmptyText::parse(content.to_owned()).map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::MalformedStream,
                    "OpenRouter content was empty after parse",
                )
            })?;
            outputs.push(ProviderOutput::Content {
                event_type: "assistant_message".to_owned(),
                content,
            });
        }
        if let Some(content) = choice
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            && !content.is_empty()
        {
            let content = NonEmptyText::parse(content.to_owned()).map_err(|_| {
                OpenRouterError::new(
                    OpenRouterErrorKind::MalformedStream,
                    "OpenRouter message content was empty after parse",
                )
            })?;
            outputs.push(ProviderOutput::Content {
                event_type: "assistant_message".to_owned(),
                content,
            });
        }
    }
    Ok(outputs)
}

/// Extracts usage from a terminal non-stream or final stream-adjacent payload.
#[must_use]
pub fn extract_usage(value: &Value) -> UsageSummary {
    let mut summary = UsageSummary::default();
    if let Some(usage) = value.get("usage") {
        summary.prompt_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        summary.completion_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if let Some(cost) = usage.get("cost").and_then(Value::as_f64) {
            summary.cost_usd_micros = f64_to_usd_micros(cost);
        } else if let Some(cost) = usage.get("total_cost").and_then(Value::as_f64) {
            summary.cost_usd_micros = f64_to_usd_micros(cost);
        }
    }
    if summary.cost_usd_micros == 0 {
        // Conservative local estimate: $0.50 / 1M input, $1.50 / 1M output.
        let input = summary.prompt_tokens.saturating_mul(500_000) / 1_000_000;
        let output = summary.completion_tokens.saturating_mul(1_500_000) / 1_000_000;
        summary.cost_usd_micros = input.saturating_add(output).max(1);
    }
    summary
}

fn f64_to_usd_micros(cost_usd: f64) -> u64 {
    if !cost_usd.is_finite() || cost_usd <= 0.0 {
        return 0;
    }
    let micros = (cost_usd * 1_000_000.0).round();
    if micros <= 0.0 {
        return 0;
    }
    // Bound before cast so precision loss cannot overflow `u64`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    {
        let max_safe = (u64::MAX / 4) as f64;
        if micros >= max_safe {
            u64::MAX / 4
        } else {
            micros as u64
        }
    }
}

/// Splits an SSE body into data payloads.
///
/// # Errors
///
/// Returns when the body is not valid UTF-8.
pub fn split_sse_data(body: &[u8]) -> Result<Vec<String>, OpenRouterError> {
    let text = std::str::from_utf8(body).map_err(|_| {
        OpenRouterError::new(
            OpenRouterErrorKind::MalformedStream,
            "OpenRouter stream is not valid UTF-8",
        )
    })?;
    let mut payloads = Vec::new();
    for block in text.split("\n\n") {
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(data) = line.strip_prefix("data:") {
                payloads.push(data.trim_start().to_owned());
            } else if let Some(data) = line.strip_prefix("data: ") {
                payloads.push(data.to_owned());
            }
        }
    }
    if payloads.is_empty() && !text.trim().is_empty() {
        // Non-SSE JSON completion body.
        payloads.push(text.trim().to_owned());
    }
    Ok(payloads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_delta_content() {
        let outputs =
            normalize_sse_data(r#"{"choices":[{"delta":{"content":"hello"}}]}"#).expect("delta");
        assert_eq!(outputs.len(), 1);
    }

    #[test]
    fn extracts_usage_cost() {
        let usage = extract_usage(&json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "cost": 0.012_345
            }
        }));
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.cost_usd_micros, 12_345);
    }
}
