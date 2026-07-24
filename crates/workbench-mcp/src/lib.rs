//! Daemon-owned MCP lifecycle, pin verification, and tool gateway.

#![forbid(unsafe_code)]

mod error;
mod gateway;
mod http;
mod pin;
mod policy;
mod redaction;
mod stdio;

pub use error::{McpError, McpErrorKind};
pub use gateway::{
    McpGateway, SharedMcpGateway, ToolAuditFact, ToolInvokeOutcome, ToolInvokeRequest,
    allows_automatic_retry, permission_label, user_deny_authoritative,
};
pub use http::{DEFAULT_MAX_RESPONSE_BYTES, FakeHttpMode, FakeHttpTransport, HttpMcpClient};
pub use pin::{
    HttpIdentity, PinStatus, canonicalize_mcp_executable, http_endpoint_sha256, require_available,
    verify_registry,
};
pub use policy::{
    ResolvedToolAccess, ToolPolicyContext, gate_before_transport, resolve_mcp_tool_access,
};
pub use redaction::{PublicToolEvent, ToolLifecycle, contains_marker, scrub_value};
pub use stdio::{ChildShutdownReport, SharedStdioPool, StdioChild, StdioPool};

/// Default maximum encoded MCP frame or HTTP response size.
pub const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
