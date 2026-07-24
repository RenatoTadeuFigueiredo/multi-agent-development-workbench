use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    MAX_BODY_BYTES, OpenRouterError, OpenRouterErrorKind,
    protocol::{UsageSummary, extract_usage, normalize_sse_data, split_sse_data},
};

/// Offline-injectable HTTP behavior for tests.
#[derive(Clone, Debug)]
pub enum FakeHttpMode {
    Stream {
        events: Vec<String>,
        usage: Value,
    },
    Oversized {
        bytes: usize,
    },
    InvalidUtf8,
    TruncatedSse,
    InvalidJson,
    TransportError,
    MidStreamFailure {
        events: Vec<String>,
    },
}

/// In-process OpenRouter HTTP transport used by default tests.
#[derive(Clone, Default)]
pub struct FakeOpenRouterTransport {
    modes: Arc<Mutex<BTreeMap<String, FakeHttpMode>>>,
    calls: Arc<AtomicUsize>,
    last_authorization: Arc<Mutex<Option<String>>>,
}

impl FakeOpenRouterTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a response mode for one route key (usually "chat").
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn set_mode(&self, route: impl Into<String>, mode: FakeHttpMode) {
        self.modes
            .lock()
            .expect("fake transport mutex")
            .insert(route.into(), mode);
    }

    #[must_use]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    /// Returns the last Authorization header value observed (tests only).
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    #[must_use]
    pub fn last_authorization(&self) -> Option<String> {
        self.last_authorization
            .lock()
            .expect("auth mutex")
            .clone()
    }

    pub(crate) fn chat_completion(
        &self,
        authorization: &str,
        _model: &str,
        _prompt: &str,
    ) -> Result<TransportResult, OpenRouterError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self
            .last_authorization
            .lock()
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "transport unavailable")
            })? = Some(authorization.to_owned());

        let mode = self
            .modes
            .lock()
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Unavailable, "transport unavailable")
            })?
            .get("chat")
            .cloned()
            .unwrap_or_else(|| FakeHttpMode::Stream {
                events: vec![
                    r#"{"choices":[{"delta":{"content":"offline "}}]}"#.to_owned(),
                    r#"{"choices":[{"delta":{"content":"openrouter"}}]}"#.to_owned(),
                ],
                usage: json!({
                    "prompt_tokens": 8,
                    "completion_tokens": 4,
                    "cost": 0.0001
                }),
            });

        match mode {
            FakeHttpMode::Stream { events, usage } => {
                let mut body = String::new();
                for event in &events {
                    body.push_str("data: ");
                    body.push_str(event);
                    body.push_str("\n\n");
                }
                body.push_str("data: [DONE]\n\n");
                if body.len() > MAX_BODY_BYTES {
                    return Err(OpenRouterError::new(
                        OpenRouterErrorKind::ResponseTooLarge,
                        "OpenRouter response exceeds the encoded body ceiling",
                    ));
                }
                Ok(TransportResult {
                    body: body.into_bytes(),
                    usage: extract_usage(&json!({ "usage": usage })),
                })
            }
            FakeHttpMode::Oversized { bytes } => {
                if bytes > MAX_BODY_BYTES {
                    Err(OpenRouterError::new(
                        OpenRouterErrorKind::ResponseTooLarge,
                        "OpenRouter response exceeds the encoded body ceiling",
                    ))
                } else {
                    Ok(TransportResult {
                        body: vec![b'x'; bytes],
                        usage: UsageSummary {
                            cost_usd_micros: 1,
                            ..UsageSummary::default()
                        },
                    })
                }
            }
            FakeHttpMode::InvalidUtf8 => Ok(TransportResult {
                body: vec![0xff, 0xfe, 0xfd],
                usage: UsageSummary::default(),
            }),
            FakeHttpMode::TruncatedSse => Ok(TransportResult {
                body: b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"\n".to_vec(),
                usage: UsageSummary::default(),
            }),
            FakeHttpMode::InvalidJson => Ok(TransportResult {
                body: b"data: {not-json}\n\n".to_vec(),
                usage: UsageSummary::default(),
            }),
            FakeHttpMode::TransportError => Err(OpenRouterError::new(
                OpenRouterErrorKind::Transport,
                "OpenRouter transport failed",
            )),
            FakeHttpMode::MidStreamFailure { events } => {
                let mut body = String::new();
                for event in &events {
                    body.push_str("data: ");
                    body.push_str(event);
                    body.push_str("\n\n");
                }
                // Incomplete: no [DONE] and no terminal usage.
                Ok(TransportResult {
                    body: body.into_bytes(),
                    usage: UsageSummary::default(),
                    // Signal incomplete via zero usage and incomplete marker handled by adapter.
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransportResult {
    pub body: Vec<u8>,
    pub usage: UsageSummary,
}

/// HTTP transport used by the adapter (offline fake by default).
#[derive(Clone)]
pub struct OpenRouterTransport {
    fake: FakeOpenRouterTransport,
    use_fake: bool,
    base_url: String,
}

impl OpenRouterTransport {
    #[must_use]
    pub fn offline(base_url: impl Into<String>) -> Self {
        Self {
            fake: FakeOpenRouterTransport::new(),
            use_fake: true,
            base_url: base_url.into(),
        }
    }

    #[must_use]
    pub fn fake(&self) -> &FakeOpenRouterTransport {
        &self.fake
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Performs a Chat Completions request.
    ///
    /// # Errors
    ///
    /// Returns redacted transport or validation failures. Live HTTPS is not
    /// used unless explicitly enabled in a future live path; default is fake.
    pub fn chat_completion(
        &self,
        secret: &Zeroizing<String>,
        model: &str,
        prompt: &str,
    ) -> Result<TransportResult, OpenRouterError> {
        let authorization = format!("Bearer {}", secret.as_str());
        if self.use_fake || self.base_url.starts_with("fake://") {
            return self
                .fake
                .chat_completion(&authorization, model, prompt);
        }
        // Live path is intentionally not wired into default builds; ignored live
        // tests can inject a fake or loopback. Refuse public network by default.
        Err(OpenRouterError::new(
            OpenRouterErrorKind::Unavailable,
            "live OpenRouter HTTP is disabled outside opt-in tests",
        ))
    }
}

/// Decodes a transport body into normalized outputs and whether it completed.
///
/// # Errors
///
/// Returns when the body is oversized or malformed.
pub fn decode_body(
    body: &[u8],
) -> Result<(Vec<workbench_core::ports::ProviderOutput>, bool), OpenRouterError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(OpenRouterError::new(
            OpenRouterErrorKind::ResponseTooLarge,
            "OpenRouter response exceeds the encoded body ceiling",
        ));
    }
    let payloads = split_sse_data(body)?;
    let mut outputs = Vec::new();
    let mut saw_done = false;
    for payload in payloads {
        if payload.trim() == "[DONE]" {
            saw_done = true;
            continue;
        }
        outputs.extend(normalize_sse_data(&payload)?);
    }
    Ok((outputs, saw_done))
}
