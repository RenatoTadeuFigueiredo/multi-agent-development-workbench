//! Pinned HTTP MCP client with size and redirect bounds.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use workbench_config::model::McpServer;

use crate::{
    error::{
        McpError, redirect_rejected, response_too_large, transport_failed, unavailable,
    },
    pin::HttpIdentity,
};

/// Default encoded response ceiling (8 MiB).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Offline-injectable HTTP behavior for tests and fakes.
#[derive(Clone, Debug)]
pub enum FakeHttpMode {
    Success { body: Value },
    Oversized { bytes: usize },
    Redirect { location: String },
    Hang,
    TransportError,
}

/// In-process HTTP transport used by default tests (no sockets).
#[derive(Clone, Default)]
pub struct FakeHttpTransport {
    modes: Arc<Mutex<BTreeMap<String, FakeHttpMode>>>,
    calls: Arc<AtomicUsize>,
}

impl FakeHttpTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_mode(&self, server_id: impl Into<String>, mode: FakeHttpMode) {
        self.modes
            .lock()
            .expect("fake http mutex")
            .insert(server_id.into(), mode);
    }

    #[must_use]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    pub(crate) fn invoke(
        &self,
        server_id: &str,
        identity: &HttpIdentity,
        max_bytes: usize,
    ) -> Result<Value, McpError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mode = self
            .modes
            .lock()
            .expect("fake http mutex")
            .get(server_id)
            .cloned()
            .unwrap_or(FakeHttpMode::Success {
                body: json!({"ok": true}),
            });
        match mode {
            FakeHttpMode::Success { body } => {
                let encoded = serde_json::to_vec(&body).map_err(|_| transport_failed())?;
                if encoded.len() > max_bytes {
                    return Err(response_too_large());
                }
                Ok(body)
            }
            FakeHttpMode::Oversized { bytes } => {
                if bytes > max_bytes {
                    Err(response_too_large())
                } else {
                    Ok(json!({"ok": true}))
                }
            }
            FakeHttpMode::Redirect { location } => {
                if identity.matches_redirect(&location) {
                    Ok(json!({"ok": true, "redirected": true}))
                } else {
                    Err(redirect_rejected())
                }
            }
            FakeHttpMode::Hang => Err(McpError::new(
                crate::error::McpErrorKind::Timeout,
                "MCP HTTP call timed out",
            )),
            FakeHttpMode::TransportError => Err(transport_failed()),
        }
    }
}

/// HTTP MCP client that prefers offline fakes and loopback TCP only.
#[derive(Clone, Default)]
pub struct HttpMcpClient {
    fake: FakeHttpTransport,
    use_fake: bool,
}

impl HttpMcpClient {
    #[must_use]
    pub fn offline() -> Self {
        Self {
            fake: FakeHttpTransport::new(),
            use_fake: true,
        }
    }

    #[must_use]
    pub fn with_loopback() -> Self {
        Self {
            fake: FakeHttpTransport::new(),
            use_fake: false,
        }
    }

    #[must_use]
    pub fn fake(&self) -> &FakeHttpTransport {
        &self.fake
    }

    /// Invokes a tool on the pinned HTTP endpoint.
    ///
    /// # Errors
    ///
    /// Returns when the endpoint is unusable, oversized, or redirects away.
    pub async fn invoke(
        &self,
        server_id: &str,
        server: &McpServer,
        operation: &str,
        arguments: &Value,
    ) -> Result<Value, McpError> {
        let url = server.url.as_deref().ok_or_else(unavailable)?;
        let identity = HttpIdentity::parse(url)?;
        if !identity.allows_connection() {
            return Err(unavailable());
        }
        let max_bytes = server
            .max_response_bytes
            .map_or(DEFAULT_MAX_RESPONSE_BYTES, |value| {
                usize::try_from(value).unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
            })
            .min(DEFAULT_MAX_RESPONSE_BYTES);

        if self.use_fake {
            return self.fake.invoke(server_id, &identity, max_bytes);
        }

        if !identity.loopback {
            // Non-loopback HTTPS requires a TLS stack; fail closed until one is
            // composed. Acceptance uses offline fakes.
            return Err(unavailable());
        }

        invoke_loopback_http(&identity, operation, arguments, max_bytes).await
    }
}

async fn invoke_loopback_http(
    identity: &HttpIdentity,
    operation: &str,
    arguments: &Value,
    max_bytes: usize,
) -> Result<Value, McpError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": operation,
            "arguments": arguments,
        }
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|_| transport_failed())?;
    if body_bytes.len() > max_bytes {
        return Err(response_too_large());
    }
    let host_port = format!("{}:{}", identity.host, identity.port);
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&host_port))
        .await
        .map_err(|_| transport_failed())?
        .map_err(|_| transport_failed())?;
    let mut stream = stream;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        identity.path,
        identity.host,
        body_bytes.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| transport_failed())?;
    stream
        .write_all(&body_bytes)
        .await
        .map_err(|_| transport_failed())?;
    stream.flush().await.map_err(|_| transport_failed())?;

    let mut response = Vec::new();
    let mut buf = [0_u8; 8_192];
    loop {
        let read = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf))
            .await
            .map_err(|_| {
                McpError::new(crate::error::McpErrorKind::Timeout, "MCP HTTP call timed out")
            })?
            .map_err(|_| transport_failed())?;
        if read == 0 {
            break;
        }
        if response.len().saturating_add(read) > max_bytes.saturating_add(4_096) {
            return Err(response_too_large());
        }
        response.extend_from_slice(&buf[..read]);
    }
    parse_http_response(&response, identity, max_bytes)
}

fn parse_http_response(
    raw: &[u8],
    identity: &HttpIdentity,
    max_bytes: usize,
) -> Result<Value, McpError> {
    let text = std::str::from_utf8(raw).map_err(|_| transport_failed())?;
    let (header, body) = text.split_once("\r\n\r\n").ok_or_else(transport_failed)?;
    let status_line = header.lines().next().ok_or_else(transport_failed)?;
    if status_line.contains(" 301 ")
        || status_line.contains(" 302 ")
        || status_line.contains(" 307 ")
        || status_line.contains(" 308 ")
    {
        let location = header.lines().find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("location:")
                .map(|value| value.trim().to_owned())
        });
        let Some(location) = location else {
            return Err(redirect_rejected());
        };
        if !identity.matches_redirect(&location) {
            return Err(redirect_rejected());
        }
        return Err(transport_failed());
    }
    if body.len() > max_bytes {
        return Err(response_too_large());
    }
    let value: Value = serde_json::from_str(body).map_err(|_| transport_failed())?;
    if value.get("error").is_some() {
        return Err(transport_failed());
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbench_config::model::McpTransport;

    fn server(url: &str, sha: &str) -> McpServer {
        McpServer {
            transport: McpTransport::Http,
            version: "1.0.0".to_owned(),
            sha256: sha.to_owned(),
            executable: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: Some(url.to_owned()),
            headers: BTreeMap::new(),
            max_response_bytes: Some(64),
        }
    }

    #[tokio::test]
    async fn fake_rejects_oversized_response() {
        let client = HttpMcpClient::offline();
        client.fake().set_mode(
            "http-a",
            FakeHttpMode::Oversized { bytes: 128 },
        );
        let sha = crate::pin::http_endpoint_sha256("http://127.0.0.1:9/mcp").expect("sha");
        let err = client
            .invoke(
                "http-a",
                &server("http://127.0.0.1:9/mcp", &sha),
                "read",
                &json!({}),
            )
            .await
            .expect_err("oversized");
        assert_eq!(err.kind(), crate::error::McpErrorKind::ResponseTooLarge);
    }

    #[tokio::test]
    async fn fake_rejects_unpinned_redirect() {
        let client = HttpMcpClient::offline();
        client.fake().set_mode(
            "http-a",
            FakeHttpMode::Redirect {
                location: "http://evil.example/mcp".to_owned(),
            },
        );
        let sha = crate::pin::http_endpoint_sha256("http://127.0.0.1:9/mcp").expect("sha");
        let err = client
            .invoke(
                "http-a",
                &server("http://127.0.0.1:9/mcp", &sha),
                "read",
                &json!({}),
            )
            .await
            .expect_err("redirect");
        assert_eq!(err.kind(), crate::error::McpErrorKind::RedirectRejected);
    }
}
