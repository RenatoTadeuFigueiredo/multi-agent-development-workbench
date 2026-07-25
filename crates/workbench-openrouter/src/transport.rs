use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use crate::{
    MAX_BODY_BYTES, OpenRouterError, OpenRouterErrorKind,
    protocol::{UsageSummary, extract_usage, normalize_sse_data, split_sse_data},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_mins(1);

/// Offline-injectable HTTP behavior for `OpenRouter` fake transport tests.
#[derive(Clone, Debug)]
pub enum FakeHttpMode {
    Stream { events: Vec<String>, usage: Value },
    Oversized { bytes: usize },
    InvalidUtf8,
    TruncatedSse,
    InvalidJson,
    TransportError,
    MidStreamFailure { events: Vec<String> },
}

/// In-process `OpenRouter` HTTP transport used by default tests.
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
        self.last_authorization.lock().expect("auth mutex").clone()
    }

    pub(crate) fn chat_completion(
        &self,
        authorization: &str,
        _model: &str,
        _prompt: &str,
    ) -> Result<TransportResult, OpenRouterError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.last_authorization.lock().map_err(|_| {
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

    /// Live HTTPS Chat Completions against `base_url` using rustls + native roots.
    #[must_use]
    pub fn live_https(base_url: impl Into<String>) -> Self {
        Self {
            fake: FakeOpenRouterTransport::new(),
            use_fake: false,
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

    #[must_use]
    pub const fn uses_fake(&self) -> bool {
        self.use_fake
    }

    /// Performs a Chat Completions request.
    ///
    /// # Errors
    ///
    /// Returns redacted transport or validation failures. Default composition
    /// uses the offline fake; live HTTPS is available via [`Self::live_https`].
    pub async fn chat_completion(
        &self,
        secret: &Zeroizing<String>,
        model: &str,
        prompt: &str,
    ) -> Result<TransportResult, OpenRouterError> {
        let authorization = format!("Bearer {}", secret.as_str());
        if self.use_fake || self.base_url.starts_with("fake://") {
            return self.fake.chat_completion(&authorization, model, prompt);
        }
        live_chat_completion(&self.base_url, &authorization, model, prompt).await
    }
}

fn default_client_config() -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs();
    for cert in certs.certs {
        let _ = roots.add(cert);
    }
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

async fn live_chat_completion(
    base_url: &str,
    authorization: &str,
    model: &str,
    prompt: &str,
) -> Result<TransportResult, OpenRouterError> {
    let (host, port, path) = parse_live_endpoint(base_url)?;
    let body = json!({
        "model": model,
        "stream": true,
        "messages": [{ "role": "user", "content": prompt }]
    })
    .to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: {authorization}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response = live_https_exchange(&host, port, &request).await?;
    let body = split_http_body(&response)?;
    if body.len() > MAX_BODY_BYTES {
        return Err(OpenRouterError::new(
            OpenRouterErrorKind::ResponseTooLarge,
            "OpenRouter response exceeds the encoded body ceiling",
        ));
    }
    let usage = extract_usage_from_sse(body);
    Ok(TransportResult {
        body: body.to_vec(),
        usage,
    })
}

fn parse_live_endpoint(base_url: &str) -> Result<(String, u16, String), OpenRouterError> {
    // Parse https://host[:port][/path] without a URL crate dependency.
    let stripped = base_url.strip_prefix("https://").ok_or_else(|| {
        OpenRouterError::new(
            OpenRouterErrorKind::InvalidConfig,
            "live OpenRouter base_url must use https://",
        )
    })?;
    let (host_port, base_path) = match stripped.split_once('/') {
        Some((host_port, rest)) => (host_port, format!("/{rest}")),
        None => (stripped, String::new()),
    };
    let host = host_port
        .split(':')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OpenRouterError::new(
                OpenRouterErrorKind::InvalidConfig,
                "OpenRouter base_url host is missing",
            )
        })?
        .to_owned();
    let port = host_port
        .split_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(443);
    let path = if base_path.ends_with("/chat/completions") {
        base_path
    } else if base_path.ends_with('/') {
        format!("{base_path}chat/completions")
    } else if base_path.is_empty() {
        "/api/v1/chat/completions".to_owned()
    } else {
        format!("{base_path}/chat/completions")
    };
    Ok((host, port, path))
}

async fn live_https_exchange(
    host: &str,
    port: u16,
    request: &str,
) -> Result<Vec<u8>, OpenRouterError> {
    let server_name = ServerName::try_from(host.to_owned()).map_err(|_| {
        OpenRouterError::new(
            OpenRouterErrorKind::InvalidConfig,
            "OpenRouter base_url host is not a valid TLS server name",
        )
    })?;
    let connector = TlsConnector::from(default_client_config());
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            OpenRouterError::new(
                OpenRouterErrorKind::Transport,
                "OpenRouter connect timed out",
            )
        })?
        .map_err(|_| {
            OpenRouterError::new(OpenRouterErrorKind::Transport, "OpenRouter connect failed")
        })?;
    let mut tls = connector.connect(server_name, stream).await.map_err(|_| {
        OpenRouterError::new(
            OpenRouterErrorKind::Transport,
            "OpenRouter TLS handshake failed",
        )
    })?;
    tls.write_all(request.as_bytes()).await.map_err(|_| {
        OpenRouterError::new(OpenRouterErrorKind::Transport, "OpenRouter write failed")
    })?;
    tls.flush().await.map_err(|_| {
        OpenRouterError::new(OpenRouterErrorKind::Transport, "OpenRouter flush failed")
    })?;
    let mut response = Vec::new();
    let mut buf = [0_u8; 8192];
    loop {
        let read = tokio::time::timeout(READ_TIMEOUT, tls.read(&mut buf))
            .await
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Transport, "OpenRouter read timed out")
            })?
            .map_err(|_| {
                OpenRouterError::new(OpenRouterErrorKind::Transport, "OpenRouter read failed")
            })?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buf[..read]);
        if response.len() > MAX_BODY_BYTES {
            return Err(OpenRouterError::new(
                OpenRouterErrorKind::ResponseTooLarge,
                "OpenRouter response exceeds the encoded body ceiling",
            ));
        }
    }
    Ok(response)
}

fn split_http_body(response: &[u8]) -> Result<&[u8], OpenRouterError> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            OpenRouterError::new(
                OpenRouterErrorKind::Transport,
                "OpenRouter response missing header terminator",
            )
        })?;
    Ok(&response[separator + 4..])
}

fn extract_usage_from_sse(body: &[u8]) -> UsageSummary {
    let Ok(payloads) = split_sse_data(body) else {
        return UsageSummary::default();
    };
    for payload in payloads {
        if payload.trim() == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(payload.trim()) {
            let usage = extract_usage(&value);
            if usage.prompt_tokens > 0
                || usage.completion_tokens > 0
                || value.get("usage").is_some()
            {
                return usage;
            }
        }
    }
    UsageSummary::default()
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
