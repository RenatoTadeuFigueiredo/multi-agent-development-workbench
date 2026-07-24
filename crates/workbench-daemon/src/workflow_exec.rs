//! Durable multi-stage workflow helpers for daemon composition (Feature 008).

use serde_json::{Value, json};
use uuid::Uuid;
use workbench_config::{
    WorkbenchConfiguration,
    model::{Workflow, WorkflowStep},
};
use workbench_core::{
    value::{NonEmptyText, WorkflowId},
    workflow::{WorkflowPhase, WorkflowRun},
};
use workbench_storage::PersistedEvent;

/// Explicit findings marker accepted from offline fakes and supervised adapters.
pub const FINDINGS_MARKER: &str = "findings_present=true";

/// Builds a new run for a validated workflow id.
///
/// # Errors
///
/// Returns when the workflow is missing, empty, or bounds are illegal.
pub fn start_run(
    config: &WorkbenchConfiguration,
    workflow_id: &str,
    run_id: Uuid,
) -> Result<(WorkflowRun, Workflow), String> {
    let workflow = config
        .workflows
        .get(workflow_id)
        .cloned()
        .ok_or_else(|| format!("workflow {workflow_id} is not configured"))?;
    if workflow.steps.is_empty() {
        return Err("workflow has no steps".to_owned());
    }
    let step_ids = workflow
        .steps
        .iter()
        .map(|step| {
            NonEmptyText::parse(step.id.clone()).map_err(|_| "workflow step id is empty".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first = &workflow.steps[0];
    let max_iterations = first.max_iterations.unwrap_or(1).clamp(1, 8);
    let on_findings_step_index = first
        .on_findings
        .as_ref()
        .and_then(|target| workflow.steps.iter().position(|step| &step.id == target));
    let run = WorkflowRun::start(
        WorkflowId::parse(workflow_id.to_owned())
            .map_err(|_| "workflow id is invalid".to_owned())?,
        NonEmptyText::parse(run_id.to_string()).map_err(|_| "run id is empty".to_owned())?,
        step_ids,
        max_iterations,
        on_findings_step_index,
    )
    .map_err(|error| error.message().to_owned())?;
    Ok((run, workflow))
}

/// Rebuilds the latest durable run snapshot from sequenced events.
#[must_use]
pub fn recover_run(history: &[PersistedEvent]) -> Option<WorkflowRun> {
    history.iter().rev().find_map(|event| {
        if event.kind != "workflow_transition" {
            return None;
        }
        event
            .payload
            .get("run")
            .and_then(|value| serde_json::from_value::<WorkflowRun>(value.clone()).ok())
    })
}

/// Reads the optional workflow id pinned on session creation.
#[must_use]
pub fn session_workflow_id(history: &[PersistedEvent]) -> Option<String> {
    history
        .iter()
        .find(|event| event.kind == "session_created")
        .and_then(|event| {
            event
                .payload
                .get("workflow_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

/// Collects the latest redirect instruction for the next dispatch.
#[must_use]
pub fn latest_redirect_instruction(history: &[PersistedEvent]) -> Option<String> {
    history.iter().rev().find_map(|event| {
        if event.kind != "session_redirected" {
            return None;
        }
        event
            .payload
            .get("instruction")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// Returns the configured step for the active run index.
#[must_use]
pub fn active_step<'a>(workflow: &'a Workflow, run: &WorkflowRun) -> Option<&'a WorkflowStep> {
    workflow.steps.get(run.active_step_index)
}

/// Effective `max_iterations` for the active step, defaulting to 1.
#[must_use]
pub fn step_max_iterations(step: &WorkflowStep) -> u32 {
    step.max_iterations.unwrap_or(1).clamp(1, 8)
}

/// Syncs run correction bounds from the active step configuration.
pub fn refresh_step_bounds(run: &mut WorkflowRun, step: &WorkflowStep, workflow: &Workflow) {
    run.max_iterations = step_max_iterations(step);
    run.on_findings_step_index = step.on_findings.as_ref().and_then(|target| {
        workflow
            .steps
            .iter()
            .position(|candidate| &candidate.id == target)
    });
}

/// Builds a redacted public transition payload.
#[must_use]
pub fn transition_payload(run: &WorkflowRun, reason: &str) -> Value {
    let step_id = run.active_step_id().map_or("none", NonEmptyText::as_str);
    json!({
        "workflow_id": run.workflow_id.as_str(),
        "run_id": run.run_id.as_str(),
        "step_id": step_id,
        "step_index": run.active_step_index,
        "iteration": run.iteration,
        "phase": phase_name(run.phase),
        "reason": reason,
        "run": run,
    })
}

/// Serializes the closed phase set for wire and storage.
#[must_use]
pub const fn phase_name(phase: WorkflowPhase) -> &'static str {
    match phase {
        WorkflowPhase::Planned => "planned",
        WorkflowPhase::Running => "running",
        WorkflowPhase::Paused => "paused",
        WorkflowPhase::AwaitingHuman => "awaiting_human",
        WorkflowPhase::Completed => "completed",
        WorkflowPhase::Cancelled => "cancelled",
        WorkflowPhase::Failed => "failed",
    }
}

/// Detects the explicit offline findings signal.
#[must_use]
pub fn content_has_findings(content: &str) -> bool {
    content.contains(FINDINGS_MARKER)
}

/// Intersects role tools with an optional step allowlist.
#[must_use]
pub fn effective_step_tools(role_tools: &[String], step_tools: &[String]) -> Vec<String> {
    if step_tools.is_empty() {
        return role_tools.to_vec();
    }
    role_tools
        .iter()
        .filter(|tool| step_tools.iter().any(|allowed| allowed == *tool))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbench_config::model::{Role, Workflow, WorkflowStep};

    fn sample_config() -> WorkbenchConfiguration {
        let mut config = WorkbenchConfiguration::safe_builtins();
        config.roles.insert(
            "spec".to_owned(),
            Role {
                model: "fake-default".to_owned(),
                tools: vec![],
                data_sources: vec![],
                required_capabilities: vec![],
                fallback_models: vec![],
            },
        );
        config.roles.insert(
            "review".to_owned(),
            Role {
                model: "fake-default".to_owned(),
                tools: vec![],
                data_sources: vec![],
                required_capabilities: vec![],
                fallback_models: vec![],
            },
        );
        config.workflows.insert(
            "primary".to_owned(),
            Workflow {
                steps: vec![
                    WorkflowStep {
                        id: "specify".to_owned(),
                        role: "spec".to_owned(),
                        on_findings: None,
                        max_iterations: None,
                        tools: vec![],
                        fallbacks: vec![],
                    },
                    WorkflowStep {
                        id: "review".to_owned(),
                        role: "review".to_owned(),
                        on_findings: Some("specify".to_owned()),
                        max_iterations: Some(2),
                        tools: vec![],
                        fallbacks: vec![],
                    },
                ],
            },
        );
        config
    }

    #[test]
    fn start_and_recover_round_trip() {
        let config = sample_config();
        let (run, _) = start_run(&config, "primary", Uuid::now_v7()).expect("start");
        let payload = transition_payload(&run, "started");
        let event = PersistedEvent {
            event_id: Uuid::now_v7(),
            session_id: Uuid::now_v7(),
            sequence: 1,
            causation_request_id: None,
            occurred_at: time::OffsetDateTime::now_utc(),
            kind: "workflow_transition".to_owned(),
            attempt_id: None,
            effect_class: None,
            payload,
        };
        let recovered = recover_run(&[event]).expect("recover");
        assert_eq!(recovered.workflow_id.as_str(), "primary");
        assert_eq!(
            recovered.active_step_id().map(NonEmptyText::as_str),
            Some("specify")
        );
    }

    #[test]
    fn step_tools_intersect_role_grant() {
        let role = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(
            effective_step_tools(&role, &["b".to_owned()]),
            vec!["b".to_owned()]
        );
        assert_eq!(effective_step_tools(&role, &[]), role);
    }
}
