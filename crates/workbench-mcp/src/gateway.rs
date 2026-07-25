//! Daemon-owned MCP and tool gateway.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;
use workbench_config::{
    WorkbenchLock,
    model::{McpTransport, ToolKind, WorkbenchConfiguration},
};
use workbench_core::{
    AttemptId,
    attempt::{Attempt, AttemptProgress},
    policy::{PermissionMode, PolicySource},
};

use crate::{
    error::{McpError, McpErrorKind, policy_denied, shutting_down, unavailable},
    http::HttpMcpClient,
    pin::{PinStatus, require_available, verify_registry},
    policy::{ToolPolicyContext, gate_before_transport, resolve_mcp_tool_access},
    redaction::{PublicToolEvent, ToolLifecycle},
    stdio::StdioPool,
};

/// Redacted audit fact recorded for a tool attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAuditFact {
    pub event: PublicToolEvent,
    pub transport: Option<&'static str>,
}

/// Outcome of a gateway invoke after policy and optional transport.
#[derive(Debug, Clone)]
pub struct ToolInvokeOutcome {
    pub attempt_id: AttemptId,
    pub progress: AttemptProgress,
    pub public: PublicToolEvent,
    pub content: Option<Value>,
    pub retried: bool,
}

/// Request to invoke a governed MCP or built-in tool through the gateway.
#[derive(Debug, Clone)]
pub struct ToolInvokeRequest {
    pub tool_id: String,
    pub operation: String,
    pub role_id: Option<String>,
    pub workflow_id: Option<String>,
    pub workflow_step_id: Option<String>,
    pub session_denied: BTreeSet<String>,
    /// `None` = not yet decided; `Some(true)` grant; `Some(false)` deny.
    pub approval_granted: Option<bool>,
    pub arguments: Value,
    pub correlation_id: String,
    /// When true, force cancel-after-start semantics for tests.
    pub simulate_cancel_after_start: bool,
    /// When true, force a pre-start failure for retry tests.
    pub simulate_pre_start_failure: bool,
    /// When true, force a post-start crash for uncertainty tests.
    pub simulate_post_start_crash: bool,
}

impl Default for ToolInvokeRequest {
    fn default() -> Self {
        Self {
            tool_id: String::new(),
            operation: String::new(),
            role_id: None,
            workflow_id: None,
            workflow_step_id: None,
            session_denied: BTreeSet::new(),
            approval_granted: None,
            arguments: Value::Object(serde_json::Map::new()),
            correlation_id: Uuid::now_v7().to_string(),
            simulate_cancel_after_start: false,
            simulate_pre_start_failure: false,
            simulate_post_start_crash: false,
        }
    }
}

/// Central MCP gateway composed by the daemon.
pub struct McpGateway {
    config: WorkbenchConfiguration,
    pins: BTreeMap<String, PinStatus>,
    runtime_dir: PathBuf,
    workspace_key: String,
    stdio: StdioPool,
    http: HttpMcpClient,
    audit: Mutex<Vec<ToolAuditFact>>,
    shutting_down: AtomicBool,
}

impl McpGateway {
    /// Verifies pins and prepares the gateway without launching children.
    ///
    /// # Errors
    ///
    /// Returns when lock verification fails structurally.
    pub fn bootstrap(
        config: WorkbenchConfiguration,
        lock: &WorkbenchLock,
        runtime_dir: impl Into<PathBuf>,
        workspace_key: impl Into<String>,
        offline_http: bool,
    ) -> Result<Self, McpError> {
        let statuses = verify_registry(&config.mcp_servers, lock)?;
        let pins = statuses
            .into_iter()
            .map(|status| (status.server_id.clone(), status))
            .collect();
        Ok(Self {
            config,
            pins,
            runtime_dir: runtime_dir.into(),
            workspace_key: workspace_key.into(),
            stdio: StdioPool::new(),
            http: if offline_http {
                HttpMcpClient::offline()
            } else {
                HttpMcpClient::with_network()
            },
            audit: Mutex::new(Vec::new()),
            shutting_down: AtomicBool::new(false),
        })
    }

    #[must_use]
    pub fn http_fake(&self) -> &crate::http::FakeHttpTransport {
        self.http.fake()
    }

    #[must_use]
    pub fn pins(&self) -> &BTreeMap<String, PinStatus> {
        &self.pins
    }

    #[must_use]
    pub fn server_available(&self, server_id: &str) -> bool {
        self.pins
            .get(server_id)
            .is_some_and(|status| status.available)
    }

    #[must_use]
    pub fn available_servers(&self) -> Vec<&PinStatus> {
        self.pins
            .values()
            .filter(|status| status.available)
            .collect()
    }

    /// Records and returns redacted audit facts.
    pub async fn audit_log(&self) -> Vec<ToolAuditFact> {
        self.audit.lock().await.clone()
    }

    /// Invokes a tool with policy, approval, pin, and transport enforcement.
    ///
    /// # Errors
    ///
    /// Returns redacted gateway failures; never includes secrets.
    #[allow(clippy::too_many_lines)]
    pub async fn invoke(&self, request: ToolInvokeRequest) -> Result<ToolInvokeOutcome, McpError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(shutting_down());
        }

        let session_refs = request
            .session_denied
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let access = resolve_mcp_tool_access(&ToolPolicyContext {
            config: &self.config,
            tool_id: &request.tool_id,
            operation: &request.operation,
            role_id: request.role_id.as_deref(),
            workflow_id: request.workflow_id.as_deref(),
            workflow_step_id: request.workflow_step_id.as_deref(),
            session_denied: session_refs,
        })?;

        let mut attempt = Attempt::plan(request.operation.clone(), access.operation.clone())
            .map_err(|_| crate::error::invalid_configuration())?;
        let attempt_id = attempt.id();
        let planned = PublicToolEvent::new(
            request.tool_id.clone(),
            ToolLifecycle::Planned,
            "planned",
            attempt_id.to_string(),
            request.correlation_id.clone(),
        )
        .with_policy_source(access.decision.authoritative_source);
        self.push_audit(ToolAuditFact {
            event: planned,
            transport: None,
        })
        .await;

        if let Err(error) = gate_before_transport(&access, request.approval_granted) {
            let lifecycle = match error.kind() {
                McpErrorKind::ApprovalRequired => ToolLifecycle::ApprovalRequired,
                McpErrorKind::ApprovalDenied | McpErrorKind::PolicyDenied => ToolLifecycle::Denied,
                _ => ToolLifecycle::Failed,
            };
            let public = PublicToolEvent::new(
                request.tool_id.clone(),
                lifecycle,
                error.message(),
                attempt_id.to_string(),
                request.correlation_id.clone(),
            )
            .with_policy_source(access.decision.authoritative_source)
            .with_error_kind(error.kind());
            self.push_audit(ToolAuditFact {
                event: public.clone(),
                transport: None,
            })
            .await;
            let _ = attempt.mark_failed();
            return Err(error);
        }

        if access.tool_kind != ToolKind::Mcp {
            return Err(policy_denied());
        }
        let server_id = access.mcp_server.as_deref().ok_or_else(unavailable)?;
        let pin = self.pins.get(server_id).ok_or_else(unavailable)?;
        if let Err(error) = require_available(pin) {
            let public = PublicToolEvent::new(
                request.tool_id.clone(),
                ToolLifecycle::Failed,
                error.message(),
                attempt_id.to_string(),
                request.correlation_id.clone(),
            )
            .with_error_kind(error.kind());
            self.push_audit(ToolAuditFact {
                event: public,
                transport: None,
            })
            .await;
            let _ = attempt.mark_failed();
            return Err(error);
        }
        let server = self
            .config
            .mcp_servers
            .get(server_id)
            .ok_or_else(unavailable)?;

        if request.simulate_pre_start_failure {
            if attempt.may_retry_automatically() {
                // Single automatic retry for proven pre-start idempotent reads.
                let retried = self
                    .dispatch_transport(&mut attempt, &request, server_id, server, false)
                    .await;
                return self.finish_outcome(attempt, request, retried, true).await;
            }
            let error = unavailable();
            let public = PublicToolEvent::new(
                request.tool_id.clone(),
                ToolLifecycle::Failed,
                error.message(),
                attempt_id.to_string(),
                request.correlation_id.clone(),
            )
            .with_error_kind(error.kind());
            self.push_audit(ToolAuditFact {
                event: public,
                transport: None,
            })
            .await;
            let _ = attempt.mark_failed();
            return Err(error);
        }

        let content = self
            .dispatch_transport(&mut attempt, &request, server_id, server, true)
            .await;
        self.finish_outcome(attempt, request, content, false).await
    }

    async fn dispatch_transport(
        &self,
        attempt: &mut Attempt,
        request: &ToolInvokeRequest,
        server_id: &str,
        server: &workbench_config::model::McpServer,
        record_start: bool,
    ) -> Result<Value, McpError> {
        if record_start {
            attempt
                .mark_started()
                .map_err(|_| crate::error::invalid_configuration())?;
            let started = PublicToolEvent::new(
                request.tool_id.clone(),
                ToolLifecycle::Started,
                "started",
                attempt.id().to_string(),
                request.correlation_id.clone(),
            );
            self.push_audit(ToolAuditFact {
                event: started,
                transport: Some(transport_name(server.transport)),
            })
            .await;
        }

        if request.simulate_cancel_after_start || request.simulate_post_start_crash {
            let _ = attempt.mark_outcome_unknown();
            return Err(crate::stdio::cancel_after_start());
        }

        match server.transport {
            McpTransport::Stdio => {
                self.stdio
                    .get_or_spawn(server_id, &self.workspace_key, server, &self.runtime_dir)
                    .await?;
                self.stdio
                    .invoke(
                        server_id,
                        &self.workspace_key,
                        &request.operation,
                        &request.arguments,
                    )
                    .await
            }
            McpTransport::Http => {
                self.http
                    .invoke(server_id, server, &request.operation, &request.arguments)
                    .await
            }
        }
    }

    async fn finish_outcome(
        &self,
        mut attempt: Attempt,
        request: ToolInvokeRequest,
        content: Result<Value, McpError>,
        retried: bool,
    ) -> Result<ToolInvokeOutcome, McpError> {
        match content {
            Ok(value) => {
                if attempt.progress() == AttemptProgress::Planned {
                    let _ = attempt.mark_started();
                }
                let _ = attempt.mark_completed();
                let public = PublicToolEvent::new(
                    request.tool_id,
                    ToolLifecycle::Succeeded,
                    "ok",
                    attempt.id().to_string(),
                    request.correlation_id,
                );
                self.push_audit(ToolAuditFact {
                    event: public.clone(),
                    transport: None,
                })
                .await;
                Ok(ToolInvokeOutcome {
                    attempt_id: attempt.id(),
                    progress: attempt.progress(),
                    public,
                    content: Some(value),
                    retried,
                })
            }
            Err(error) => {
                let lifecycle = if error.kind() == McpErrorKind::OutcomeUnknown {
                    let _ = attempt.mark_outcome_unknown();
                    ToolLifecycle::OutcomeUnknown
                } else {
                    let progress = attempt.progress();
                    if progress == AttemptProgress::Planned
                        || (progress.dispatch_started() && !progress.is_definite_terminal())
                    {
                        let _ = attempt.mark_failed();
                    }
                    ToolLifecycle::Failed
                };
                let public = PublicToolEvent::new(
                    request.tool_id,
                    lifecycle,
                    error.message(),
                    attempt.id().to_string(),
                    request.correlation_id,
                )
                .with_error_kind(error.kind());
                self.push_audit(ToolAuditFact {
                    event: public.clone(),
                    transport: None,
                })
                .await;
                if error.kind() == McpErrorKind::OutcomeUnknown {
                    Ok(ToolInvokeOutcome {
                        attempt_id: attempt.id(),
                        progress: AttemptProgress::OutcomeUnknown,
                        public,
                        content: None,
                        retried,
                    })
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn push_audit(&self, fact: ToolAuditFact) {
        self.audit.lock().await.push(fact);
    }

    /// Rejects new work and reaps supervised children.
    ///
    /// # Errors
    ///
    /// Returns when a child cannot be reaped.
    pub async fn shutdown(&self) -> Result<(), McpError> {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.stdio.shutdown(None).await
    }

    /// Reaps only this gateway workspace's children.
    ///
    /// # Errors
    ///
    /// Returns when a workspace child cannot be reaped.
    pub async fn shutdown_workspace(&self) -> Result<(), McpError> {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.stdio.shutdown(Some(&self.workspace_key)).await
    }

    #[must_use]
    pub async fn active_stdio_children(&self) -> usize {
        self.stdio.active_count().await
    }

    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    #[must_use]
    pub fn workspace_key(&self) -> &str {
        &self.workspace_key
    }

    #[must_use]
    pub fn config(&self) -> &WorkbenchConfiguration {
        &self.config
    }

    /// Ensures a stdio server is spawned for isolation tests.
    ///
    /// # Errors
    ///
    /// Returns when the server is unknown, unpinned, or spawn fails.
    pub async fn ensure_stdio(&self, server_id: &str) -> Result<(), McpError> {
        let server = self
            .config
            .mcp_servers
            .get(server_id)
            .ok_or_else(unavailable)?;
        let pin = self.pins.get(server_id).ok_or_else(unavailable)?;
        require_available(pin)?;
        self.stdio
            .get_or_spawn(server_id, &self.workspace_key, server, &self.runtime_dir)
            .await
    }
}

const fn transport_name(transport: McpTransport) -> &'static str {
    match transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}

/// Shared gateway handle.
pub type SharedMcpGateway = Arc<McpGateway>;

/// Whether automatic retry is permitted for the attempt progress.
#[must_use]
pub fn allows_automatic_retry(attempt: &Attempt) -> bool {
    attempt.may_retry_automatically()
}

/// Builds a denial event when repository tries to widen a user deny.
#[must_use]
pub fn user_deny_authoritative() -> PolicySource {
    PolicySource::User
}

/// Maps permission mode for diagnostics.
#[must_use]
pub const fn permission_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::ApprovalRequired => "approval-required",
        PermissionMode::Denied => "denied",
    }
}
