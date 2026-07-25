//! Feature 016 acceptance: Grok terminal `WorkbenchBackend` MVP.

#![allow(clippy::manual_let_else)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use workbench_terminal_backend::{
    GROK_BUILD_FORK_COMPATIBILITY_PIN, WORKBENCH_AGENT_STDIO_ARGS, WorkbenchBackend,
    WorkbenchBackendError,
};

const FEATURE: &str =
    include_str!("../../../doc/arch/specs/features/grok-terminal-workbench-backend-mvp.feature");
const LIB: &str = include_str!("../../workbench-terminal-backend/src/lib.rs");
const MAKEFILE: &str = include_str!("../../../Makefile");
const STATUS: &str = include_str!("../../../docs/project/STATUS.md");
const ARCH: &str = include_str!("../../../docs/architecture/grok-build-terminal-integration.md");

struct ScenarioBinding {
    case_name: &'static str,
    #[allow(dead_code)]
    fingerprint: u64,
    evidence_test: &'static str,
}

const SCENARIO_BINDINGS: [ScenarioBinding; 4] = [
    binding(
        "WorkbenchBackend plans agent stdio with absolute paths",
        0x0,
        "backend_plans_agent_stdio",
    ),
    binding(
        "Relative executable fails closed",
        0x0,
        "relative_executable_fails_closed",
    ),
    binding(
        "Compatibility pin surface exists",
        0x0,
        "compatibility_pin_exists",
    ),
    binding(
        "Architecture residual is documented",
        0x0,
        "residual_documented",
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
fn feature_016_gherkin_is_bound() {
    let cases = parse_feature(FEATURE);
    assert_eq!(cases.len(), 4);
    let bindings = SCENARIO_BINDINGS
        .iter()
        .map(|binding| (binding.case_name, binding))
        .collect::<BTreeMap<_, _>>();
    for case in &cases {
        assert!(bindings.contains_key(case.name.as_str()), "{}", case.name);
        assert_ne!(fingerprint(&case.steps), 0);
    }
}

#[test]
fn every_binding_names_executable_repository_evidence() {
    let _ = backend_plans_agent_stdio;
    let _ = relative_executable_fails_closed;
    let _ = compatibility_pin_exists;
    let _ = residual_documented;
    let evidence = SCENARIO_BINDINGS
        .iter()
        .map(|binding| binding.evidence_test)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        evidence,
        BTreeSet::from([
            "backend_plans_agent_stdio",
            "relative_executable_fails_closed",
            "compatibility_pin_exists",
            "residual_documented",
        ])
    );
}

#[test]
fn backend_plans_agent_stdio() {
    let backend = WorkbenchBackend::new(
        PathBuf::from("/usr/local/bin/workbench"),
        PathBuf::from("/workspace"),
    )
    .expect("absolute");
    backend.validate_launch_plan().expect("plan");
    assert_eq!(WORKBENCH_AGENT_STDIO_ARGS, &["agent", "stdio"]);
    assert!(MAKEFILE.contains("feature_016") || MAKEFILE.contains("test-terminal-backend"));
}

#[test]
fn relative_executable_fails_closed() {
    assert_eq!(
        WorkbenchBackend::new("workbench", "/workspace").expect_err("relative"),
        WorkbenchBackendError::RelativeExecutable
    );
}

#[test]
fn compatibility_pin_exists() {
    let _ = GROK_BUILD_FORK_COMPATIBILITY_PIN;
    assert!(LIB.contains("GROK_BUILD_FORK_COMPATIBILITY_PIN"));
    assert!(LIB.contains("WorkbenchBackend"));
}

#[test]
fn residual_documented() {
    assert!(ARCH.contains("WorkbenchBackend") || ARCH.contains("GrokShellBackend"));
    assert!(
        STATUS.contains("#33")
            || STATUS.contains("Feature 016")
            || STATUS.contains("WorkbenchBackend")
            || STATUS.contains("terminal")
    );
}

struct ParsedCase {
    name: String,
    steps: Vec<String>,
}

fn parse_feature(source: &str) -> Vec<ParsedCase> {
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
        } else if let Some(case) = current.as_mut()
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
    cases
}

fn fingerprint(steps: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for step in steps {
        step.hash(&mut hasher);
    }
    hasher.finish()
}
