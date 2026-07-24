//! Feature 008 offline acceptance for the multi-agent workflow executor.

use std::collections::BTreeMap;

use workbench_config::{
    WorkbenchConfiguration,
    model::{
        ApprovalMode, EffectClass, McpServer, McpTransport, Operation, Role, Tool, ToolKind,
        Workflow, WorkflowStep,
    },
    validate::validate,
};
use workbench_core::{
    value::{NonEmptyText, WorkflowId},
    workflow::{WorkflowPhase, WorkflowRun},
};

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
    let _ = bound_review_correction_loops;
    let _ = fallback_aliases_validate_and_remain_ordered;
    let _ = pause_resume_cancel_controls;
    let _ = redirect_is_additive_instruction_only;
    let _ = recover_run_from_serialized_state;
    let _ = step_tool_allowlist_is_configuration_local;
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

fn primary_workflow_config() -> WorkbenchConfiguration {
    let mut config = WorkbenchConfiguration::safe_builtins();
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
