//! Role and workflow allowlists layered on monotonic policy intersection.

use std::collections::{BTreeMap, BTreeSet};

use workbench_config::model::{
    ApprovalMode, DefaultToolMode, EffectClass as ConfigEffectClass, Operation, Tool, ToolKind,
    WorkbenchConfiguration,
};
use workbench_core::{
    attempt::{EffectClass, OperationPolicy},
    policy::{
        PermissionMode, PolicyDecision, PolicyLayer, PolicySource, protect_effect,
        resolve_tool_policy,
    },
    value::ToolId,
};

// PermissionMode ranking is private in workbench-core; mirror it for approval.

use crate::error::{McpError, approval_required, invalid_configuration, policy_denied};

/// Inputs that produce the effective MCP tool decision.
#[derive(Debug, Clone)]
pub struct ToolPolicyContext<'a> {
    pub config: &'a WorkbenchConfiguration,
    pub tool_id: &'a str,
    pub operation: &'a str,
    pub role_id: Option<&'a str>,
    pub workflow_id: Option<&'a str>,
    pub workflow_step_id: Option<&'a str>,
    pub session_denied: BTreeSet<&'a str>,
}

/// Explainable policy resolution for one tool operation.
#[derive(Debug, Clone)]
pub struct ResolvedToolAccess {
    pub decision: PolicyDecision,
    pub operation: OperationPolicy,
    pub approval_mode: ApprovalMode,
    pub tool_kind: ToolKind,
    pub mcp_server: Option<String>,
}

/// Builds intersecting layers and applies effect-class protection.
///
/// # Errors
///
/// Returns when the tool or operation is unknown.
pub fn resolve_mcp_tool_access(
    ctx: &ToolPolicyContext<'_>,
) -> Result<ResolvedToolAccess, McpError> {
    let tool = mcp_tool(ctx.config, ctx.tool_id).ok_or_else(policy_denied)?;
    let operation = tool_operation(tool, ctx.operation).ok_or_else(policy_denied)?;
    let tool_id = ToolId::parse(ctx.tool_id).map_err(|_| invalid_configuration())?;
    let layers = build_layers(ctx, &tool_id);
    let base = resolve_tool_policy(&tool_id, &layers);
    let effect = map_effect(operation.effect_class);
    let protected = protect_effect(base, effect, operation.material_cost);
    let with_approval = apply_approval_mode(protected, operation.approval);
    Ok(ResolvedToolAccess {
        decision: with_approval,
        operation: OperationPolicy {
            effect_class: effect,
            explicitly_idempotent: operation.idempotent,
            material_cost: operation.material_cost,
        },
        approval_mode: operation.approval,
        tool_kind: tool.kind,
        mcp_server: tool.mcp_server.clone(),
    })
}

fn apply_approval_mode(current: PolicyDecision, mode: ApprovalMode) -> PolicyDecision {
    match mode {
        ApprovalMode::Never | ApprovalMode::Policy => current,
        ApprovalMode::Always => {
            let forced = current.mode.intersect(PermissionMode::ApprovalRequired);
            if forced == current.mode {
                current
            } else {
                PolicyDecision {
                    mode: forced,
                    authoritative_source: PolicySource::EffectClass,
                }
            }
        }
    }
}

trait IntersectExt {
    fn intersect(self, other: Self) -> Self;
}

impl IntersectExt for PermissionMode {
    fn intersect(self, other: Self) -> Self {
        // Mirror workbench_core ranking without exposing private method.
        let rank = |mode: PermissionMode| match mode {
            PermissionMode::ReadOnly => 0_u8,
            PermissionMode::ApprovalRequired => 1,
            PermissionMode::Denied => 2,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

fn build_layers(ctx: &ToolPolicyContext<'_>, tool_id: &ToolId) -> Vec<PolicyLayer> {
    let mut layers = Vec::new();

    // Built-in: unlisted tools are denied; registered tools start read-only.
    let mut built_in_modes = BTreeMap::new();
    for name in ctx.config.tools.keys() {
        if let Ok(id) = ToolId::parse(name.clone()) {
            built_in_modes.insert(id, PermissionMode::ReadOnly);
        }
    }
    layers.push(PolicyLayer {
        source: PolicySource::BuiltIn,
        default_mode: PermissionMode::Denied,
        tool_modes: built_in_modes,
        denied_tools: BTreeSet::new(),
    });

    let user_denied = ctx
        .config
        .policies
        .global_deny
        .iter()
        .filter_map(|name| ToolId::parse(name.clone()).ok())
        .collect::<BTreeSet<_>>();
    layers.push(PolicyLayer {
        source: PolicySource::User,
        default_mode: map_default_mode(ctx.config.policies.default_tool_mode),
        tool_modes: BTreeMap::new(),
        denied_tools: user_denied,
    });

    // Repository layer: only present as default mode; cannot clear user deny.
    layers.push(PolicyLayer {
        source: PolicySource::Repository,
        default_mode: map_default_mode(ctx.config.policies.default_tool_mode),
        tool_modes: BTreeMap::new(),
        denied_tools: BTreeSet::new(),
    });

    let session_denied = ctx
        .session_denied
        .iter()
        .filter_map(|name| ToolId::parse((*name).to_owned()).ok())
        .collect::<BTreeSet<_>>();
    layers.push(PolicyLayer {
        source: PolicySource::Session,
        default_mode: PermissionMode::ReadOnly,
        tool_modes: BTreeMap::new(),
        denied_tools: session_denied,
    });

    if let Some(role_id) = ctx.role_id
        && let Some(role) = ctx.config.roles.get(role_id)
    {
        let allowed = role
            .tools
            .iter()
            .filter_map(|name| ToolId::parse(name.clone()).ok())
            .collect::<BTreeSet<_>>();
        let mut denied = BTreeSet::new();
        if !allowed.contains(tool_id) {
            denied.insert(tool_id.clone());
        }
        // Empty role allowlist denies every tool.
        if allowed.is_empty() {
            denied.insert(tool_id.clone());
        }
        layers.push(PolicyLayer {
            source: PolicySource::Role,
            default_mode: PermissionMode::Denied,
            tool_modes: allowed
                .into_iter()
                .map(|id| (id, PermissionMode::ReadOnly))
                .collect(),
            denied_tools: denied,
        });
    }

    if let (Some(workflow_id), Some(step_id)) = (ctx.workflow_id, ctx.workflow_step_id)
        && let Some(workflow) = ctx.config.workflows.get(workflow_id)
        && let Some(step) = workflow.steps.iter().find(|step| step.id == step_id)
        && !step.tools.is_empty()
    {
        let allowed = step
            .tools
            .iter()
            .filter_map(|name| ToolId::parse(name.clone()).ok())
            .collect::<BTreeSet<_>>();
        let mut denied = BTreeSet::new();
        if !allowed.contains(tool_id) {
            denied.insert(tool_id.clone());
        }
        layers.push(PolicyLayer {
            source: PolicySource::Workflow,
            default_mode: PermissionMode::Denied,
            tool_modes: allowed
                .into_iter()
                .map(|id| (id, PermissionMode::ReadOnly))
                .collect(),
            denied_tools: denied,
        });
    }

    layers
}

const fn map_default_mode(mode: DefaultToolMode) -> PermissionMode {
    match mode {
        DefaultToolMode::ReadOnly => PermissionMode::ReadOnly,
        DefaultToolMode::ApprovalRequired => PermissionMode::ApprovalRequired,
        DefaultToolMode::Denied => PermissionMode::Denied,
    }
}

const fn map_effect(effect: ConfigEffectClass) -> EffectClass {
    match effect {
        ConfigEffectClass::IdempotentRead => EffectClass::IdempotentRead,
        ConfigEffectClass::IdempotentWrite => EffectClass::IdempotentWrite,
        ConfigEffectClass::PaidInference => EffectClass::PaidInference,
        ConfigEffectClass::NonIdempotentWrite => EffectClass::NonIdempotentWrite,
        ConfigEffectClass::Production => EffectClass::Production,
        ConfigEffectClass::Credential => EffectClass::Credential,
    }
}

/// Converts a resolved access decision into a gate result before transport.
///
/// # Errors
///
/// Returns policy denial or approval-required without starting transport.
pub fn gate_before_transport(
    access: &ResolvedToolAccess,
    granted: Option<bool>,
) -> Result<(), McpError> {
    match access.decision.mode {
        PermissionMode::Denied => Err(policy_denied()),
        PermissionMode::ApprovalRequired => match granted {
            Some(true) => Ok(()),
            Some(false) => Err(crate::error::approval_denied()),
            None => Err(approval_required()),
        },
        PermissionMode::ReadOnly => Ok(()),
    }
}

/// Locates an MCP-backed tool definition.
pub fn mcp_tool<'a>(config: &'a WorkbenchConfiguration, tool_id: &str) -> Option<&'a Tool> {
    config
        .tools
        .get(tool_id)
        .filter(|tool| tool.kind == ToolKind::Mcp)
}

/// Locates one named operation.
pub fn tool_operation<'a>(tool: &'a Tool, operation: &str) -> Option<&'a Operation> {
    tool.operations.iter().find(|item| item.name == operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbench_config::model::{
        ApprovalMode, EffectClass as ConfigEffectClass, McpServer, McpTransport, Operation, Tool,
        ToolKind, Workflow, WorkflowStep,
    };

    fn base_config() -> WorkbenchConfiguration {
        let mut config = WorkbenchConfiguration::safe_builtins();
        config.mcp_servers.insert(
            "repo".to_owned(),
            McpServer {
                transport: McpTransport::Stdio,
                version: "1.0.0".to_owned(),
                sha256: "a".repeat(64),
                executable: Some("/tmp/fake".to_owned()),
                args: Vec::new(),
                env: BTreeMap::new(),
                url: None,
                headers: BTreeMap::new(),
                max_response_bytes: None,
            },
        );
        config.tools.insert(
            "repo-read".to_owned(),
            Tool {
                kind: ToolKind::Mcp,
                mcp_server: Some("repo".to_owned()),
                operations: vec![Operation {
                    name: "read".to_owned(),
                    effect_class: ConfigEffectClass::IdempotentRead,
                    idempotent: true,
                    material_cost: false,
                    approval: ApprovalMode::Never,
                }],
            },
        );
        config.tools.insert(
            "cluster-mutate".to_owned(),
            Tool {
                kind: ToolKind::Mcp,
                mcp_server: Some("repo".to_owned()),
                operations: vec![Operation {
                    name: "apply".to_owned(),
                    effect_class: ConfigEffectClass::NonIdempotentWrite,
                    idempotent: false,
                    material_cost: false,
                    approval: ApprovalMode::Policy,
                }],
            },
        );
        config.tools.insert(
            "prod-deploy".to_owned(),
            Tool {
                kind: ToolKind::Mcp,
                mcp_server: Some("repo".to_owned()),
                operations: vec![Operation {
                    name: "deploy".to_owned(),
                    effect_class: ConfigEffectClass::Production,
                    idempotent: false,
                    material_cost: false,
                    approval: ApprovalMode::Policy,
                }],
            },
        );
        config.roles.insert(
            "reviewer".to_owned(),
            workbench_config::model::Role {
                model: "fake-default".to_owned(),
                tools: vec!["repo-read".to_owned()],
                data_sources: Vec::new(),
                required_capabilities: Vec::new(),
                fallback_models: Vec::new(),
            },
        );
        config.roles.insert(
            "builder".to_owned(),
            workbench_config::model::Role {
                model: "fake-default".to_owned(),
                tools: vec!["repo-read".to_owned(), "cluster-mutate".to_owned()],
                data_sources: Vec::new(),
                required_capabilities: Vec::new(),
                fallback_models: Vec::new(),
            },
        );
        config.workflows.insert(
            "ship".to_owned(),
            Workflow {
                steps: vec![WorkflowStep {
                    id: "review".to_owned(),
                    role: "builder".to_owned(),
                    on_findings: None,
                    max_iterations: None,
                    tools: vec!["repo-read".to_owned()],
                    fallbacks: Vec::new(),
                }],
            },
        );
        config
    }

    #[test]
    fn role_allowlist_denies_unlisted_tool() {
        let config = base_config();
        let access = resolve_mcp_tool_access(&ToolPolicyContext {
            config: &config,
            tool_id: "cluster-mutate",
            operation: "apply",
            role_id: Some("reviewer"),
            workflow_id: None,
            workflow_step_id: None,
            session_denied: BTreeSet::new(),
        })
        .expect("access");
        assert_eq!(access.decision.mode, PermissionMode::Denied);
        assert_eq!(access.decision.authoritative_source, PolicySource::Role);
    }

    #[test]
    fn workflow_allowlist_narrows_role() {
        let config = base_config();
        let access = resolve_mcp_tool_access(&ToolPolicyContext {
            config: &config,
            tool_id: "cluster-mutate",
            operation: "apply",
            role_id: Some("builder"),
            workflow_id: Some("ship"),
            workflow_step_id: Some("review"),
            session_denied: BTreeSet::new(),
        })
        .expect("access");
        assert_eq!(access.decision.mode, PermissionMode::Denied);
        assert_eq!(access.decision.authoritative_source, PolicySource::Workflow);
    }

    #[test]
    fn repository_cannot_widen_user_deny() {
        let mut config = base_config();
        config.policies.global_deny.push("prod-deploy".to_owned());
        let access = resolve_mcp_tool_access(&ToolPolicyContext {
            config: &config,
            tool_id: "prod-deploy",
            operation: "deploy",
            role_id: None,
            workflow_id: None,
            workflow_step_id: None,
            session_denied: BTreeSet::new(),
        })
        .expect("access");
        assert_eq!(access.decision.mode, PermissionMode::Denied);
        assert_eq!(access.decision.authoritative_source, PolicySource::User);
    }

    #[test]
    fn protected_effects_require_approval() {
        let config = base_config();
        let access = resolve_mcp_tool_access(&ToolPolicyContext {
            config: &config,
            tool_id: "cluster-mutate",
            operation: "apply",
            role_id: Some("builder"),
            workflow_id: None,
            workflow_step_id: None,
            session_denied: BTreeSet::new(),
        })
        .expect("access");
        assert_eq!(access.decision.mode, PermissionMode::ApprovalRequired);
        assert!(gate_before_transport(&access, None).is_err());
        assert!(gate_before_transport(&access, Some(true)).is_ok());
        assert!(gate_before_transport(&access, Some(false)).is_err());
    }
}
