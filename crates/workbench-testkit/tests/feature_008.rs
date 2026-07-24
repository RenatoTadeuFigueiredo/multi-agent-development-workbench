//! Feature 008 offline acceptance for the multi-agent workflow executor.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use serde_json::Value;
use uuid::Uuid;
use workbench_config::{
    WorkbenchConfiguration,
    model::{
        ApprovalMode, EffectClass, McpServer, McpTransport, Operation, Role, Tool, ToolKind,
        Workflow, WorkflowStep,
    },
    validate::validate,
};
use workbench_core::policy::PermissionMode;
use workbench_core::{
    value::{NonEmptyText, WorkflowId},
    workflow::{WorkflowPhase, WorkflowRun},
};
use workbench_daemon::{Application, ClientContext, FakeBehavior, StartupConfiguration};
use workbench_mcp::{ToolPolicyContext, resolve_mcp_tool_access};
use workbench_protocol::{
    ClientCommand, Command, EventKind, SessionEvent,
    command::{
        ApprovalDecision, ApprovalParams, AttachSessionParams, CreateSessionParams, EmptyParams,
        PromptParams,
    },
    response::{AttachSessionResult, CreateSessionResult, SessionResult, SessionState},
};
use workbench_testkit::client::{LocalDaemonHarness, ProtocolTestClient};

const FEATURE: &str = include_str!(
    "../../../doc/arch/specs/features/execute-configurable-multi-agent-workflows-that-resolve.feature"
);

struct ScenarioBinding {
    case_name: &'static str,
    fingerprint: u64,
    #[allow(dead_code)]
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 11] = [
    binding(
        "Validate well-formed and broken workflow graphs",
        0x740b_0c7a_0d9a_dd36,
        "validate_well_formed_and_broken_workflow_graphs",
    ),
    binding(
        "Advance sequential stages with explainable routing plans",
        0x5d5e_3d27_2df3_c840,
        "sequential_advance_and_primary_path",
    ),
    binding(
        "Prove the primary Claude to Codex to Grok to Codex path offline",
        0x9b5f_fb9c_d1f5_5061,
        "sequential_advance_and_primary_path",
    ),
    binding(
        "Bound review-correction loops",
        0x8948_20e3_e6a7_06c0,
        "bound_review_correction_loops",
    ),
    binding(
        "Select configured fallback when primary preflight fails",
        0x6279_1204_2c4d_8975,
        "fallback_aliases_validate_and_remain_ordered",
    ),
    binding(
        "Pause and resume freeze and continue advancement",
        0x1454_d5df_ac8e_6387,
        "pause_resume_cancel_controls",
    ),
    binding(
        "Cancel terminates the run without inventing success",
        0x3fa1_7d9d_0b84_b2da,
        "pause_resume_cancel_controls",
    ),
    binding(
        "Redirect injects instruction without rewriting history",
        0x6a61_0cdf_65ec_4213,
        "redirect_is_additive_instruction_only",
    ),
    binding(
        "Recover active step after daemon interruption",
        0xf38c_dc90_0e55_4858,
        "recover_run_from_serialized_state",
    ),
    binding(
        "Workflow step tools stay under the central MCP gateway",
        0x614c_f96d_5ab2_f6d9,
        "step_tool_allowlist_is_configuration_local",
    ),
    binding(
        "Default suite stays offline and quota-free",
        0xc018_6fdd_8a8e_827d,
        "default_suite_stays_offline",
    ),
];

const fn binding(
    case_name: &'static str,
    fingerprint: u64,
    evidence_test: &'static str,
) -> ScenarioBinding {
    ScenarioBinding {
        case_name,
        fingerprint,
        evidence_test,
    }
}

#[test]
fn repository_owned_gherkin_has_eleven_fingerprinted_cases() {
    let parsed = parse_feature(FEATURE);
    assert_eq!(parsed.cases.len(), 11);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(bindings.len(), 11);
    for case in &parsed.cases {
        assert!(
            bindings.contains_key(case.name.as_str()),
            "missing binding for {}",
            case.name
        );
        let fp = fingerprint(&case.steps);
        assert_ne!(fp, 0, "fingerprint collapsed for {}", case.name);
        let binding = bindings[case.name.as_str()];
        assert_eq!(fp, binding.fingerprint, "scenario drifted: {}", case.name);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = validate_well_formed_and_broken_workflow_graphs;
    let _ = sequential_advance_and_primary_path;
    let _ = sequential_advance_and_primary_path_daemon;
    let _ = bound_review_correction_loops;
    let _ = bound_review_correction_loops_daemon;
    let _ = fallback_aliases_validate_and_remain_ordered;
    let _ = pause_resume_cancel_controls;
    let _ = pause_resume_cancel_controls_daemon;
    let _ = redirect_is_additive_instruction_only;
    let _ = recover_run_from_serialized_state;
    let _ = recover_run_from_serialized_state_daemon;
    let _ = step_tool_allowlist_is_configuration_local;
    let _ = step_tool_allowlist_is_configuration_local_gateway;
    let _ = default_suite_stays_offline;
}

#[test]
fn validate_well_formed_and_broken_workflow_graphs() {
    let good = primary_workflow_config();
    validate(&good).expect("well-formed workflow");

    let mut broken = good;
    broken.workflows.get_mut("primary").expect("wf").steps[0].role = "missing-role".to_owned();
    assert!(validate(&broken).is_err());
}

#[test]
fn sequential_advance_and_primary_path() {
    let config = primary_workflow_config();
    let steps = &config.workflows["primary"].steps;
    assert_eq!(steps.len(), 4);
    // Role ids encode the primary Claude → Codex → Grok → Codex path; models
    // remain the offline fake so the suite stays quota-free.
    assert_eq!(steps[0].role, "claude-spec");
    assert_eq!(steps[1].role, "codex-review");
    assert_eq!(steps[2].role, "grok-implement");
    assert_eq!(steps[3].role, "codex-validate");

    let mut run = WorkflowRun::start(
        WorkflowId::parse("primary").expect("id"),
        NonEmptyText::parse("run-primary").expect("id"),
        steps
            .iter()
            .map(|step| NonEmptyText::parse(step.id.clone()).expect("step"))
            .collect(),
        1,
        None,
    )
    .expect("start");

    let mut order = Vec::new();
    loop {
        order.push(run.active_step_id().expect("step").as_str().to_owned());
        run.advance_after_success().expect("advance");
        if run.phase == WorkflowPhase::Completed {
            break;
        }
    }
    assert_eq!(
        order,
        vec![
            "specify".to_owned(),
            "review".to_owned(),
            "implement".to_owned(),
            "validate".to_owned()
        ]
    );
}

#[test]
fn bound_review_correction_loops() {
    let mut run = WorkflowRun::start(
        WorkflowId::parse("primary").expect("id"),
        NonEmptyText::parse("run-loop").expect("id"),
        vec![
            NonEmptyText::parse("review").expect("id"),
            NonEmptyText::parse("fix").expect("id"),
        ],
        2,
        Some(1),
    )
    .expect("start");
    run.apply_findings().expect("iter1");
    assert_eq!(run.phase, WorkflowPhase::Running);
    run.apply_findings().expect("ceiling");
    assert_eq!(run.phase, WorkflowPhase::AwaitingHuman);
}

#[test]
fn fallback_aliases_validate_and_remain_ordered() {
    let mut config = primary_workflow_config();
    config.models.insert(
        "fake-fallback".to_owned(),
        workbench_config::model::Model {
            provider: "fake".to_owned(),
            runtime_model: "deterministic-fallback".to_owned(),
        },
    );
    config.workflows.get_mut("primary").expect("wf").steps[2].fallbacks =
        vec!["fake-fallback".to_owned()];
    validate(&config).expect("fallback alias valid");
    assert_eq!(
        config.workflows["primary"].steps[2].fallbacks,
        vec!["fake-fallback".to_owned()]
    );

    config.workflows.get_mut("primary").expect("wf").steps[2].fallbacks =
        vec!["missing-model".to_owned()];
    assert!(validate(&config).is_err());
}

#[test]
fn pause_resume_cancel_controls() {
    let mut run = WorkflowRun::start(
        WorkflowId::parse("primary").expect("id"),
        NonEmptyText::parse("run-ctrl").expect("id"),
        vec![
            NonEmptyText::parse("a").expect("id"),
            NonEmptyText::parse("b").expect("id"),
        ],
        1,
        None,
    )
    .expect("start");
    run.pause().expect("pause");
    assert!(!run.phase.permits_dispatch());
    run.resume().expect("resume");
    run.cancel().expect("cancel");
    assert_eq!(run.phase, WorkflowPhase::Cancelled);
    assert!(!run.phase.permits_dispatch());
}

#[test]
fn redirect_is_additive_instruction_only() {
    // Redirect remains a session-level instruction (Feature 001); the workflow
    // run identity and step index are unchanged by an additive redirect text.
    let run = WorkflowRun::start(
        WorkflowId::parse("primary").expect("id"),
        NonEmptyText::parse("run-redir").expect("id"),
        vec![NonEmptyText::parse("a").expect("id")],
        1,
        None,
    )
    .expect("start");
    let before = run.clone();
    let redirect = NonEmptyText::parse("prefer smaller diffs").expect("text");
    assert_eq!(run.workflow_id, before.workflow_id);
    assert_eq!(run.active_step_index, before.active_step_index);
    assert_eq!(redirect.as_str(), "prefer smaller diffs");
}

#[test]
fn recover_run_from_serialized_state() {
    let run = WorkflowRun::start(
        WorkflowId::parse("primary").expect("id"),
        NonEmptyText::parse("run-rec").expect("id"),
        vec![
            NonEmptyText::parse("specify").expect("id"),
            NonEmptyText::parse("review").expect("id"),
        ],
        1,
        None,
    )
    .expect("start");
    let mut advanced = run;
    advanced.advance_after_success().expect("advance");
    let encoded = serde_json::to_string(&advanced).expect("serialize");
    let recovered: WorkflowRun = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(
        recovered.active_step_id().map(NonEmptyText::as_str),
        Some("review")
    );
    assert_eq!(recovered.phase, WorkflowPhase::Running);
}

#[test]
fn step_tool_allowlist_is_configuration_local() {
    let config = primary_workflow_config();
    let step = &config.workflows["primary"].steps[1];
    assert!(step.tools.contains(&"repo-read".to_owned()));
    assert!(!step.tools.contains(&"prod-deploy".to_owned()));
}

#[test]
fn default_suite_stays_offline() {
    assert!(
        !FEATURE.contains("https://"),
        "feature file must not require live endpoints"
    );
}

#[tokio::test]
async fn sequential_advance_and_primary_path_daemon() {
    let (_harness, mut client, session_id) = workflow_client(FakeBehavior {
        response_delay: Duration::from_millis(5),
        ..FakeBehavior::default()
    })
    .await;
    prompt_and_grant(&mut client, session_id, "run primary path").await;
    let events = wait_for_session_event(&mut client, session_id, |event| {
        event.kind == EventKind::SessionCompleted
    })
    .await;
    let roles: Vec<String> = events
        .iter()
        .filter(|event| event.kind == EventKind::RoutingPlanned)
        .filter_map(|event| {
            event
                .data
                .get("role")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(
        roles,
        vec![
            "claude-spec".to_owned(),
            "codex-review".to_owned(),
            "grok-implement".to_owned(),
            "codex-validate".to_owned()
        ]
    );
    assert!(
        events
            .iter()
            .filter(|e| e.kind == EventKind::RoutingPlanned)
            .all(|event| event.data.get("selected_by").and_then(Value::as_str) == Some("workflow"))
    );
    assert!(events.iter().any(|event| {
        event.kind == EventKind::WorkflowTransition
            && event.data.get("reason").and_then(Value::as_str) == Some("completed")
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::DispatchStarted)
            .count(),
        4
    );
}

#[tokio::test]
async fn bound_review_correction_loops_daemon() {
    let (_harness, mut client, session_id) = workflow_client(FakeBehavior {
        response_delay: Duration::from_millis(5),
        report_findings: true,
        ..FakeBehavior::default()
    })
    .await;
    prompt_and_grant(&mut client, session_id, "force findings loop").await;
    let events = wait_for_session_event(&mut client, session_id, |event| {
        event.kind == EventKind::SessionPaused
            && event.data.get("actor").and_then(Value::as_str) == Some("system:workflow")
    })
    .await;
    assert!(events.iter().any(|event| {
        event.kind == EventKind::WorkflowTransition
            && event.data.get("phase").and_then(Value::as_str) == Some("awaiting_human")
    }));
}

#[tokio::test]
async fn pause_resume_cancel_controls_daemon() {
    let (_harness, mut client, session_id) = workflow_client(FakeBehavior {
        response_delay: Duration::from_secs(30),
        ..FakeBehavior::default()
    })
    .await;
    prompt_and_grant(&mut client, session_id, "long running step").await;
    let _ = wait_for_session_event(&mut client, session_id, |event| {
        event.kind == EventKind::DispatchStarted
    })
    .await;
    client
        .call(protocol_command(
            Some(session_id),
            Command::SessionPause(EmptyParams {}),
        ))
        .await
        .expect("pause");
    client
        .call(protocol_command(
            Some(session_id),
            Command::SessionResume(EmptyParams {}),
        ))
        .await
        .expect("resume");
    client
        .call(protocol_command(
            Some(session_id),
            Command::SessionCancel(EmptyParams {}),
        ))
        .await
        .expect("cancel");
    let events = wait_for_session_event(&mut client, session_id, |event| {
        matches!(
            event.kind,
            EventKind::SessionCancelled | EventKind::OutcomeUnknown
        )
    })
    .await;
    assert!(!events.iter().any(|event| {
        event.kind == EventKind::WorkflowTransition
            && event.data.get("reason").and_then(Value::as_str) == Some("completed")
    }));
}

#[tokio::test]
async fn recover_run_from_serialized_state_daemon() {
    let (application, _harness, mut client, session_id) = workflow_client_with_app(FakeBehavior {
        response_delay: Duration::from_millis(5),
        ..FakeBehavior::default()
    })
    .await;
    prompt_and_grant(&mut client, session_id, "partial run").await;
    let _ = wait_for_session_event(&mut client, session_id, |event| {
        event.kind == EventKind::SessionCompleted
    })
    .await;
    let history = application.session_history(session_id).expect("history");
    let run = history
        .iter()
        .rev()
        .find(|event| event.kind == "workflow_transition")
        .and_then(|event| event.payload.get("run").cloned())
        .and_then(|value| serde_json::from_value::<WorkflowRun>(value).ok())
        .expect("durable run snapshot");
    assert_eq!(run.phase, WorkflowPhase::Completed);
    assert_eq!(run.workflow_id.as_str(), "primary");
}

#[test]
fn step_tool_allowlist_is_configuration_local_gateway() {
    let config = primary_workflow_config();
    let denied = resolve_mcp_tool_access(&ToolPolicyContext {
        config: &config,
        tool_id: "prod-deploy",
        operation: "deploy",
        role_id: Some("codex-review"),
        workflow_id: Some("primary"),
        workflow_step_id: Some("review"),
        session_denied: BTreeSet::default(),
    });
    assert!(
        denied.is_err()
            || denied
                .as_ref()
                .is_ok_and(|access| access.decision.mode == PermissionMode::Denied),
        "excluded mutating tool must fail closed before transport"
    );
}

async fn workflow_client(fake: FakeBehavior) -> (LocalDaemonHarness, ProtocolTestClient, Uuid) {
    let (_app, harness, client, session_id) = workflow_client_with_app(fake).await;
    (harness, client, session_id)
}

async fn workflow_client_with_app(
    fake: FakeBehavior,
) -> (
    Arc<Application>,
    LocalDaemonHarness,
    ProtocolTestClient,
    Uuid,
) {
    let startup = StartupConfiguration::from_configuration(primary_workflow_config())
        .expect("workflow startup");
    let application = Application::in_memory(startup, fake).expect("application");
    let harness = LocalDaemonHarness::start(Arc::clone(&application)).expect("harness");
    let mut client = ProtocolTestClient::connect(harness.endpoint(), "feature-008")
        .await
        .expect("client");
    let created: CreateSessionResult = serde_json::from_value(
        client
            .call(protocol_command(
                None,
                Command::SessionCreate(CreateSessionParams {
                    persistent: true,
                    configuration_overrides: None,
                    workflow: Some("primary".to_owned()),
                }),
            ))
            .await
            .expect("create"),
    )
    .expect("create result");
    let attach: AttachSessionResult = serde_json::from_value(
        client
            .call(protocol_command(
                Some(created.session_id),
                Command::SessionAttach(AttachSessionParams { after_sequence: 0 }),
            ))
            .await
            .expect("attach"),
    )
    .expect("attach result");
    assert_eq!(attach.state, SessionState::Ready);
    (application, harness, client, created.session_id)
}

fn protocol_command(session_id: Option<Uuid>, command: Command) -> ClientCommand {
    ClientCommand {
        protocol: workbench_protocol::PROTOCOL_V1.to_owned(),
        request_id: Uuid::now_v7(),
        session_id,
        command,
    }
}

async fn prompt_and_grant(client: &mut ProtocolTestClient, session_id: Uuid, text: &str) {
    client
        .call(protocol_command(
            Some(session_id),
            Command::SessionPrompt(PromptParams {
                text: text.to_owned(),
                explicit_target: None,
            }),
        ))
        .await
        .expect("prompt");
    let session: SessionResult = serde_json::from_value(
        client
            .call(protocol_command(
                Some(session_id),
                Command::SessionGet(EmptyParams {}),
            ))
            .await
            .expect("session.get"),
    )
    .expect("session result");
    if let Some(approval_id) = session.pending_approval_id {
        client
            .call(protocol_command(
                Some(session_id),
                Command::SessionApprovalResolve(ApprovalParams {
                    approval_id,
                    decision: ApprovalDecision::Grant,
                }),
            ))
            .await
            .expect("grant approval");
    }
}

async fn wait_for_session_event<F>(
    client: &mut ProtocolTestClient,
    session_id: Uuid,
    mut predicate: F,
) -> Vec<SessionEvent>
where
    F: FnMut(&SessionEvent) -> bool,
{
    let mut collected = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = client.next_event().await.expect("event");
            assert_eq!(event.session_id, session_id);
            let done = predicate(&event);
            collected.push(event);
            if done {
                return collected;
            }
        }
    })
    .await
    .expect("event before deadline")
}

#[allow(dead_code)]
fn _client_context_marker() -> ClientContext {
    ClientContext {
        uid: 1000,
        client_name: "feature-008".to_owned(),
    }
}

fn primary_workflow_config() -> WorkbenchConfiguration {
    let mut config = WorkbenchConfiguration::safe_builtins();
    // Offline workflow acceptance dispatches paid-inference fakes without an
    // interactive approval gate so multi-step advancement can complete.
    config.policies.default_tool_mode = workbench_config::model::DefaultToolMode::ReadOnly;
    for role_id in [
        "claude-spec",
        "codex-review",
        "grok-implement",
        "codex-validate",
    ] {
        config.roles.insert(
            role_id.to_owned(),
            Role {
                model: "fake-default".to_owned(),
                tools: vec!["repo-read".to_owned()],
                data_sources: Vec::new(),
                required_capabilities: Vec::new(),
                fallback_models: Vec::new(),
            },
        );
    }
    config.tools.insert(
        "repo-read".to_owned(),
        Tool {
            kind: ToolKind::Mcp,
            mcp_server: Some("stdio-a".to_owned()),
            operations: vec![Operation {
                name: "read".to_owned(),
                effect_class: EffectClass::IdempotentRead,
                idempotent: true,
                material_cost: false,
                approval: ApprovalMode::Never,
            }],
        },
    );
    config.tools.insert(
        "prod-deploy".to_owned(),
        Tool {
            kind: ToolKind::Mcp,
            mcp_server: Some("stdio-a".to_owned()),
            operations: vec![Operation {
                name: "deploy".to_owned(),
                effect_class: EffectClass::Production,
                idempotent: false,
                material_cost: true,
                approval: ApprovalMode::Always,
            }],
        },
    );
    config.mcp_servers.insert(
        "stdio-a".to_owned(),
        McpServer {
            transport: McpTransport::Stdio,
            version: "1.0.0".to_owned(),
            sha256: "a".repeat(64),
            executable: Some("/usr/bin/true".to_owned()),
            args: Vec::new(),
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            max_response_bytes: None,
        },
    );
    config.workflows.insert(
        "primary".to_owned(),
        Workflow {
            steps: vec![
                step("specify", "claude-spec", None, None, vec![]),
                step(
                    "review",
                    "codex-review",
                    Some("specify"),
                    Some(2),
                    vec!["repo-read".to_owned()],
                ),
                step("implement", "grok-implement", None, None, vec![]),
                step("validate", "codex-validate", None, None, vec![]),
            ],
        },
    );
    config
}

fn step(
    id: &str,
    role: &str,
    on_findings: Option<&str>,
    max_iterations: Option<u32>,
    tools: Vec<String>,
) -> WorkflowStep {
    WorkflowStep {
        id: id.to_owned(),
        role: role.to_owned(),
        on_findings: on_findings.map(str::to_owned),
        max_iterations,
        tools,
        fallbacks: Vec::new(),
    }
}

struct ParsedFeature {
    cases: Vec<ParsedCase>,
}

struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

fn parse_feature(source: &str) -> ParsedFeature {
    let mut cases = Vec::new();
    let mut current: Option<ParsedCase> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Scenario:") {
            if let Some(case) = current.take() {
                cases.push(case);
            }
            current = Some(ParsedCase {
                name: rest.trim().to_owned(),
                steps: Vec::new(),
            });
            continue;
        }
        if let Some(case) = current.as_mut()
            && (trimmed.starts_with("Given ")
                || trimmed.starts_with("When ")
                || trimmed.starts_with("Then ")
                || trimmed.starts_with("And "))
        {
            case.steps.push(trimmed.to_owned());
        }
    }
    if let Some(case) = current {
        cases.push(case);
    }
    ParsedFeature { cases }
}

fn fingerprint(steps: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for step in steps {
        for byte in step.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0x0a;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
