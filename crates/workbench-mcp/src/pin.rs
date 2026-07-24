//! Pin verification and executable path safety for MCP artifacts.

use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use rustix::{
    fs::{Mode, OFlags},
    process::getuid,
};
use sha2::{Digest, Sha256};
use workbench_config::model::{McpServer, McpTransport};
use workbench_config::{ConfigError, WorkbenchLock};

use crate::error::{McpError, invalid_configuration, pin_mismatch, unavailable};

/// Result of verifying one MCP server against its lock pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatus {
    pub server_id: String,
    pub available: bool,
    pub transport: McpTransport,
    pub reason: Option<&'static str>,
}

/// Verifies every configured MCP against lock digests and on-disk safety.
///
/// # Errors
///
/// Returns when the lock itself is structurally invalid.
pub fn verify_registry(
    servers: &std::collections::BTreeMap<String, McpServer>,
    lock: &WorkbenchLock,
) -> Result<Vec<PinStatus>, McpError> {
    lock.verify().map_err(|_| invalid_configuration())?;
    let mut statuses = Vec::with_capacity(servers.len());
    for (name, server) in servers {
        let locked = lock.mcps.get(name);
        let status = match locked {
            None => PinStatus {
                server_id: name.clone(),
                available: false,
                transport: server.transport,
                reason: Some("missing lock pin"),
            },
            Some(pin) if pin.version != server.version || pin.sha256 != server.sha256 => {
                PinStatus {
                    server_id: name.clone(),
                    available: false,
                    transport: server.transport,
                    reason: Some("pin mismatch"),
                }
            }
            Some(_) => verify_artifact(name, server),
        };
        statuses.push(status);
    }
    Ok(statuses)
}

fn verify_artifact(name: &str, server: &McpServer) -> PinStatus {
    match server.transport {
        McpTransport::Stdio => match verify_stdio_artifact(server) {
            Ok(()) => PinStatus {
                server_id: name.to_owned(),
                available: true,
                transport: McpTransport::Stdio,
                reason: None,
            },
            Err(reason) => PinStatus {
                server_id: name.to_owned(),
                available: false,
                transport: McpTransport::Stdio,
                reason: Some(reason),
            },
        },
        McpTransport::Http => match verify_http_endpoint(server) {
            Ok(()) => PinStatus {
                server_id: name.to_owned(),
                available: true,
                transport: McpTransport::Http,
                reason: None,
            },
            Err(reason) => PinStatus {
                server_id: name.to_owned(),
                available: false,
                transport: McpTransport::Http,
                reason: Some(reason),
            },
        },
    }
}

fn verify_stdio_artifact(server: &McpServer) -> Result<(), &'static str> {
    let path = server.executable.as_deref().ok_or("missing executable")?;
    let canonical = canonicalize_mcp_executable(Path::new(path)).map_err(|_| "unsafe path")?;
    let digest = sha256_file(&canonical).map_err(|_| "missing artifact")?;
    if digest != server.sha256 {
        return Err("pin mismatch");
    }
    Ok(())
}

fn verify_http_endpoint(server: &McpServer) -> Result<(), &'static str> {
    let url = server.url.as_deref().ok_or("missing url")?;
    let identity = HttpIdentity::parse(url).map_err(|_| "invalid url")?;
    let material = identity.pin_material();
    let digest = hex::encode(Sha256::digest(material.as_bytes()));
    if digest != server.sha256 {
        return Err("pin mismatch");
    }
    if !identity.allows_connection() {
        return Err("unsupported endpoint");
    }
    Ok(())
}

/// Canonical absolute executable without symlinks or group/world write.
///
/// # Errors
///
/// Returns when the path is relative, traverses parents, is a symlink, is not
/// owned by the current user, or is group/world writable.
pub fn canonicalize_mcp_executable(path: &Path) -> Result<PathBuf, ConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::UnsafeSource(
            "MCP executable must be an absolute path without parent traversal".to_owned(),
        ));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(ConfigError::UnsafeSource(format!(
                "MCP executable {} must not contain symbolic links",
                path.display()
            )));
        }
        if metadata.is_dir() && metadata.permissions().mode() & 0o022 != 0 {
            return Err(ConfigError::UnsafeSource(format!(
                "MCP executable {} must not traverse writable directories",
                path.display()
            )));
        }
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    let descriptor = rustix::fs::open(
        &canonical,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    let file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::UnsafeSource(error.to_string()))?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(ConfigError::UnsafeSource(format!(
            "MCP executable {} must be an executable regular file without group or world write access",
            path.display()
        )));
    }
    Ok(canonical)
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, ConfigError> {
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ConfigError::Lock(error.to_string()))?;
    let mut file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| ConfigError::Lock(error.to_string()))?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(ConfigError::UnsafeSource(format!(
            "MCP executable {} became unsafe",
            path.display()
        )));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ConfigError::Lock(error.to_string()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Pinned HTTP endpoint identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpIdentity {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub loopback: bool,
}

impl HttpIdentity {
    /// Parses a restricted absolute HTTP(S) URL.
    ///
    /// # Errors
    ///
    /// Returns when the URL is not absolute HTTP/HTTPS or has an empty host.
    pub fn parse(url: &str) -> Result<Self, McpError> {
        let (scheme, rest) = url.split_once("://").ok_or_else(invalid_configuration)?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(invalid_configuration());
        }
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_owned()),
        };
        if authority.contains('@') || authority.is_empty() {
            return Err(invalid_configuration());
        }
        let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            let port: u16 = port.parse().map_err(|_| invalid_configuration())?;
            (host.to_ascii_lowercase(), port)
        } else {
            let port = if scheme == "https" { 443 } else { 80 };
            (authority.to_ascii_lowercase(), port)
        };
        if host.is_empty() || host.contains(' ') {
            return Err(invalid_configuration());
        }
        let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        if scheme == "http" && !loopback {
            return Err(invalid_configuration());
        }
        Ok(Self {
            scheme,
            host,
            port,
            path,
            loopback,
        })
    }

    #[must_use]
    pub fn pin_material(&self) -> String {
        format!("{}://{}:{}{}", self.scheme, self.host, self.port, self.path)
    }

    #[must_use]
    pub fn allows_connection(&self) -> bool {
        // Production non-loopback requires TLS (https). Loopback may use http.
        self.scheme == "https" || self.loopback
    }

    #[must_use]
    pub fn matches_redirect(&self, location: &str) -> bool {
        Self::parse(location).is_ok_and(|other| {
            other.scheme == self.scheme && other.host == self.host && other.port == self.port
        })
    }
}

/// Computes the lock digest material for an HTTP MCP URL.
///
/// # Errors
///
/// Returns when the URL is not an absolute `http(s)` endpoint.
pub fn http_endpoint_sha256(url: &str) -> Result<String, McpError> {
    let identity = HttpIdentity::parse(url)?;
    Ok(hex::encode(Sha256::digest(
        identity.pin_material().as_bytes(),
    )))
}

/// Ensures a pin status is available or maps to a stable gateway error.
///
/// # Errors
///
/// Returns pin mismatch or unavailable when the status is not available.
pub fn require_available(status: &PinStatus) -> Result<(), McpError> {
    if status.available {
        Ok(())
    } else if status.reason == Some("pin mismatch") {
        Err(pin_mismatch())
    } else {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal_in_executable() {
        let error = canonicalize_mcp_executable(Path::new("/tmp/../etc/passwd"));
        assert!(error.is_err());
    }

    #[test]
    fn parses_loopback_http_identity() {
        let identity = HttpIdentity::parse("http://127.0.0.1:9/mcp").expect("identity");
        assert!(identity.loopback);
        assert!(identity.allows_connection());
    }

    #[test]
    fn rejects_cleartext_non_loopback() {
        assert!(HttpIdentity::parse("http://example.com/mcp").is_err());
    }

    #[test]
    fn rejects_unpinned_redirect_host() {
        let identity = HttpIdentity::parse("http://127.0.0.1:9/mcp").expect("identity");
        assert!(!identity.matches_redirect("http://evil.example/mcp"));
    }
}
