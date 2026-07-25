//! Pinned HTTP/HTTPS MCP client with size and redirect bounds.

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
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;
use workbench_config::model::McpServer;

use crate::{
    error::{McpError, redirect_rejected, response_too_large, transport_failed, unavailable},
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

    /// Installs a fake response mode for one server id.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
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

/// HTTP/HTTPS MCP client that prefers offline fakes, then real TCP (+ TLS).
#[derive(Clone)]
pub struct HttpMcpClient {
    fake: FakeHttpTransport,
    use_fake: bool,
    tls: Arc<ClientConfig>,
    /// Optional TCP connect host override (identity host remains SNI / Host).
    connect_host_override: Option<String>,
}

impl Default for HttpMcpClient {
    fn default() -> Self {
        Self::offline()
    }
}

impl HttpMcpClient {
    #[must_use]
    pub fn offline() -> Self {
        Self {
            fake: FakeHttpTransport::new(),
            use_fake: true,
            tls: default_client_config(),
            connect_host_override: None,
        }
    }

    /// Production client: loopback cleartext HTTP and verified HTTPS.
    #[must_use]
    pub fn with_network() -> Self {
        Self {
            fake: FakeHttpTransport::new(),
            use_fake: false,
            tls: default_client_config(),
            connect_host_override: None,
        }
    }

    /// Backward-compatible alias for [`Self::with_network`].
    #[must_use]
    pub fn with_loopback() -> Self {
        Self::with_network()
    }

    /// Network client that trusts a custom TLS config (offline fixtures).
    #[must_use]
    pub fn with_tls_config(tls: Arc<ClientConfig>) -> Self {
        Self {
            fake: FakeHttpTransport::new(),
            use_fake: false,
            tls,
            connect_host_override: None,
        }
    }

    /// Overrides the TCP connect host while keeping identity host for SNI/Host.
    #[must_use]
    pub fn with_connect_host_override(mut self, host: impl Into<String>) -> Self {
        self.connect_host_override = Some(host.into());
        self
    }

    #[must_use]
    pub fn fake(&self) -> &FakeHttpTransport {
        &self.fake
    }

    /// Invokes a tool on the pinned HTTP(S) endpoint.
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

        if identity.scheme == "https" {
            return invoke_https(
                &identity,
                operation,
                arguments,
                max_bytes,
                Arc::clone(&self.tls),
                self.connect_host_override.as_deref(),
            )
            .await;
        }

        // Cleartext HTTP is only allowed for loopback (enforced by parse).
        invoke_cleartext_http(&identity, operation, arguments, max_bytes).await
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
    // ALPN not required for raw HTTP/1.1 MCP invoke.
    config.alpn_protocols.clear();
    Arc::new(config)
}

/// Builds a client config that trusts a single DER-encoded certificate (tests).
///
/// # Errors
///
/// Returns when the certificate cannot be added to the root store.
pub fn client_config_with_root_der(der: &[u8]) -> Result<Arc<ClientConfig>, McpError> {
    let mut roots = RootCertStore::empty();
    let cert = rustls_pki_types::CertificateDer::from(der.to_vec());
    roots.add(cert).map_err(|_| transport_failed())?;
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols.clear();
    Ok(Arc::new(config))
}

async fn invoke_cleartext_http(
    identity: &HttpIdentity,
    operation: &str,
    arguments: &Value,
    max_bytes: usize,
) -> Result<Value, McpError> {
    let host_port = format!("{}:{}", identity.host, identity.port);
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&host_port))
        .await
        .map_err(|_| transport_failed())?
        .map_err(|_| transport_failed())?;
    exchange_http(stream, identity, operation, arguments, max_bytes).await
}

async fn invoke_https(
    identity: &HttpIdentity,
    operation: &str,
    arguments: &Value,
    max_bytes: usize,
    tls: Arc<ClientConfig>,
    connect_host_override: Option<&str>,
) -> Result<Value, McpError> {
    let connect_host = connect_host_override.unwrap_or(identity.host.as_str());
    let host_port = format!("{connect_host}:{}", identity.port);
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&host_port))
        .await
        .map_err(|_| transport_failed())?
        .map_err(|_| transport_failed())?;

    let server_name = ServerName::try_from(identity.host.as_str())
        .map_err(|_| transport_failed())?
        .to_owned();
    let connector = TlsConnector::from(tls);
    let tls_stream = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(server_name, stream))
        .await
        .map_err(|_| transport_failed())?
        .map_err(|_| transport_failed())?;

    exchange_http(tls_stream, identity, operation, arguments, max_bytes).await
}

async fn exchange_http<S>(
    mut stream: S,
    identity: &HttpIdentity,
    operation: &str,
    arguments: &Value,
    max_bytes: usize,
) -> Result<Value, McpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
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
                McpError::new(
                    crate::error::McpErrorKind::Timeout,
                    "MCP HTTP call timed out",
                )
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
    use std::sync::Arc;

    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
    use rustls::{ServerConfig, pki_types::PrivateKeyDer};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;
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
            max_response_bytes: Some(4_096),
        }
    }

    #[tokio::test]
    async fn fake_rejects_oversized_response() {
        let client = HttpMcpClient::offline();
        client
            .fake()
            .set_mode("http-a", FakeHttpMode::Oversized { bytes: 8_192 });
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

    #[tokio::test]
    async fn offline_tls_fixture_serves_non_loopback_https() {
        let host = "mcp.test.invalid";
        let key_pair = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(vec![host.to_owned()]).expect("params");
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, host);
        params.subject_alt_names = vec![SanType::DnsName(host.try_into().expect("san"))];
        let cert = params.self_signed(&key_pair).expect("cert");
        let cert_der = cert.der().clone();
        let key_der = PrivateKeyDer::try_from(key_pair.serialize_der()).expect("key der");

        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)
            .expect("server config");
        server_config.alpn_protocols.clear();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server_task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            let mut tls = acceptor.accept(tcp).await.expect("tls accept");
            let mut buf = vec![0_u8; 16_384];
            let _ = tls.read(&mut buf).await;
            let body = br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true,"via":"tls"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            tls.write_all(response.as_bytes()).await.expect("hdr");
            tls.write_all(body).await.expect("body");
            tls.shutdown().await.ok();
        });

        let tls = client_config_with_root_der(cert_der.as_ref()).expect("client tls");
        let client = HttpMcpClient::with_tls_config(tls).with_connect_host_override("127.0.0.1");
        let url = format!("https://{host}:{port}/mcp");
        let sha = crate::pin::http_endpoint_sha256(&url).expect("sha");
        let identity = HttpIdentity::parse(&url).expect("identity");
        assert!(!identity.loopback, "fixture host must be non-loopback");

        let value = client
            .invoke("remote-https", &server(&url, &sha), "read", &json!({}))
            .await
            .expect("https invoke");
        assert_eq!(value["ok"], true);
        assert_eq!(value["via"], "tls");
        server_task.await.expect("server");
    }

    #[tokio::test]
    async fn https_unpinned_redirect_fails_closed() {
        let host = "mcp-redirect.test.invalid";
        let key_pair = KeyPair::generate().expect("key");
        let mut params = CertificateParams::new(vec![host.to_owned()]).expect("params");
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, host);
        params.subject_alt_names = vec![SanType::DnsName(host.try_into().expect("san"))];
        let cert = params.self_signed(&key_pair).expect("cert");
        let cert_der = cert.der().clone();
        let key_der = PrivateKeyDer::try_from(key_pair.serialize_der()).expect("key der");

        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der)
            .expect("server config");
        server_config.alpn_protocols.clear();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server_task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            let mut tls = acceptor.accept(tcp).await.expect("tls accept");
            let mut buf = vec![0_u8; 16_384];
            let _ = tls.read(&mut buf).await;
            let response = "HTTP/1.1 302 Found\r\nLocation: https://evil.example/mcp\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            tls.write_all(response.as_bytes()).await.expect("hdr");
            tls.shutdown().await.ok();
        });

        let tls = client_config_with_root_der(cert_der.as_ref()).expect("client tls");
        let client = HttpMcpClient::with_tls_config(tls).with_connect_host_override("127.0.0.1");
        let url = format!("https://{host}:{port}/mcp");
        let sha = crate::pin::http_endpoint_sha256(&url).expect("sha");
        let err = client
            .invoke("remote-https", &server(&url, &sha), "read", &json!({}))
            .await
            .expect_err("redirect");
        assert_eq!(err.kind(), crate::error::McpErrorKind::RedirectRejected);
        assert!(!format!("{err}").contains("evil"));
        server_task.await.expect("server");
    }

    #[tokio::test]
    async fn tls_failure_does_not_echo_secret_markers() {
        let marker = "TLS-SECRET-MARKER-F013";
        let tls = client_config_with_root_der(b"not-a-cert").unwrap_or_else(|_| {
            // Invalid DER fails at build; use empty roots to force handshake fail.
            let roots = RootCertStore::empty();
            let mut config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols.clear();
            Arc::new(config)
        });
        // Empty roots: handshake fails without embedding caller secrets.
        let client = HttpMcpClient::with_tls_config(tls).with_connect_host_override("127.0.0.1");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        // Accept and drop to force transport failure if anything connects.
        let _accept = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let url = format!("https://mcp.fail.invalid:{port}/mcp");
        let sha = crate::pin::http_endpoint_sha256(&url).expect("sha");
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_owned(), marker.to_owned());
        let mut srv = server(&url, &sha);
        srv.headers = headers;
        let err = client
            .invoke("fail-https", &srv, "read", &json!({ "token": marker }))
            .await
            .expect_err("tls fail");
        let surface = format!("{err:?}{err}");
        assert!(!surface.contains(marker));
    }

    /// Opt-in live smoke against a public HTTPS endpoint (not run in default CI).
    #[tokio::test]
    #[ignore = "live network HTTPS; opt-in only"]
    async fn live_public_https_handshake_smoke() {
        let client = HttpMcpClient::with_network();
        // example.com serves HTTPS; MCP JSON body may fail application-level parse,
        // but TLS + HTTP framing must not return the old non-loopback unavailable stub.
        let url = "https://example.com/";
        let sha = crate::pin::http_endpoint_sha256(url).expect("sha");
        let result = client
            .invoke("live", &server(url, &sha), "read", &json!({}))
            .await;
        match result {
            Ok(_) => {}
            Err(err) => {
                assert_ne!(
                    err.kind(),
                    crate::error::McpErrorKind::Unavailable,
                    "non-loopback HTTPS must not fail closed as unavailable once TLS is composed"
                );
            }
        }
    }
}
