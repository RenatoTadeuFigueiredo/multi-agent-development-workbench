use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchConfiguration {
    pub version: u32,
    pub providers: BTreeMap<String, Provider>,
    pub models: BTreeMap<String, Model>,
    pub roles: BTreeMap<String, Role>,
    pub tools: BTreeMap<String, Tool>,
    pub data_sources: BTreeMap<String, DataSource>,
    pub mcp_servers: BTreeMap<String, McpServer>,
    pub workflows: BTreeMap<String, Workflow>,
    pub routing: Routing,
    pub policies: Policies,
    pub storage: Storage,
    pub protocol: ProtocolLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    #[serde(rename = "type")]
    pub kind: ProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<ProviderDriver>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<Privacy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    SubscriptionCli,
    Api,
    Acp,
    Fake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderDriver {
    ClaudeCode,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    pub zero_data_retention: bool,
    pub data_collection: DataCollection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataCollection {
    Deny,
    Allow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub provider: String,
    pub runtime_model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub data_sources: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<Capability>,
    #[serde(default)]
    pub fallback_models: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Streaming,
    ToolCalling,
    StructuredOutput,
    SessionResume,
    Cancellation,
    Vision,
    Mcp,
    Acp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectClass {
    IdempotentRead,
    IdempotentWrite,
    PaidInference,
    NonIdempotentWrite,
    Production,
    Credential,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub kind: ToolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<String>,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    Builtin,
    Mcp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub name: String,
    pub effect_class: EffectClass,
    pub idempotent: bool,
    pub material_cost: bool,
    pub approval: ApprovalMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    Never,
    Policy,
    Always,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSource {
    pub tool: String,
    pub operation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    pub transport: McpTransport,
    pub version: String,
    pub sha256: String,
    /// Absolute user-owned executable path for `stdio` transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// Optional argv suffix after the executable for `stdio` transport.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Opaque secret handles (`platform:` / `keychain:` / `secret-service:`)
    /// resolved at call time for `stdio` child environment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Absolute HTTPS or loopback HTTP URL for `http` transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Opaque secret handles for HTTP header values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Optional tighter response ceiling; defaults to 8 MiB when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_response_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    Stdio,
    Http,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStep {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_findings: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
    /// Optional step allowlist that further restricts the role tool grant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    pub default_role: String,
    pub confidence_threshold: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policies {
    pub default_tool_mode: DefaultToolMode,
    pub global_deny: Vec<String>,
    pub production_mutations: ProductionMutations,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultToolMode {
    ReadOnly,
    ApprovalRequired,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProductionMutations {
    ApprovalRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    pub encryption: Encryption,
    pub retention_days: Option<u32>,
    pub export_format: ExportFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encryption {
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    #[serde(rename = "age/v1")]
    AgeV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolLimits {
    pub max_frame_bytes: u64,
    pub max_client_queue_events: u64,
    pub max_client_queue_bytes: u64,
    pub cancellation_deadline_ms: u64,
}

impl WorkbenchConfiguration {
    pub fn safe_builtins() -> Self {
        let providers = BTreeMap::from([(
            "fake".to_owned(),
            Provider {
                kind: ProviderType::Fake,
                driver: None,
                executable: None,
                credential_ref: None,
                privacy: None,
            },
        )]);
        let models = BTreeMap::from([(
            "fake-default".to_owned(),
            Model {
                provider: "fake".to_owned(),
                runtime_model: "deterministic-v1".to_owned(),
            },
        )]);
        let roles = BTreeMap::from([(
            "workspace-coordinator".to_owned(),
            Role {
                model: "fake-default".to_owned(),
                tools: Vec::new(),
                data_sources: Vec::new(),
                required_capabilities: Vec::new(),
                fallback_models: Vec::new(),
            },
        )]);

        Self {
            version: 1,
            providers,
            models,
            roles,
            tools: BTreeMap::new(),
            data_sources: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            workflows: BTreeMap::new(),
            routing: Routing {
                default_role: "workspace-coordinator".to_owned(),
                confidence_threshold: 0.85,
            },
            policies: Policies {
                default_tool_mode: DefaultToolMode::ReadOnly,
                global_deny: Vec::new(),
                production_mutations: ProductionMutations::ApprovalRequired,
            },
            storage: Storage {
                encryption: Encryption::Required,
                retention_days: None,
                export_format: ExportFormat::AgeV1,
            },
            protocol: ProtocolLimits {
                max_frame_bytes: 8_388_608,
                max_client_queue_events: 1_024,
                max_client_queue_bytes: 8_388_608,
                cancellation_deadline_ms: 5_000,
            },
        }
    }
}
